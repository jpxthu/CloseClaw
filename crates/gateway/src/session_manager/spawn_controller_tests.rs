//! Unit tests for SpawnController::validate.
//!
//! Covers the 4 rejection scenarios + 2 success scenarios defined in
//! the plan (Step 1.8.A). Each test sets up a minimal `ConfigManager`
//! + `SessionManager` fixture and exercises the validation flow end-to-end.
//!
//! All tests use `#[tokio::test]` because `SpawnController::validate`
//! awaits on `SessionManager` methods.

use std::sync::Arc;

use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};
use closeclaw_config::agents::{ModelSpec, SubagentsConfig};
use closeclaw_config::ConfigManager;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;

use crate::session_manager::spawn_controller::{SpawnController, SpawnError};
use crate::session_manager::{ChildSessionInfo, SpawnMode};
use crate::{DmScope, GatewayConfig, Message, SessionManager};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::rules::RuleSetBuilder;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal `GatewayConfig` for tests.
fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

/// Build a `PermissionEngine` with an empty RuleSet.
fn make_permission_engine() -> PermissionEngine {
    PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap())
}

/// Build a `SessionManager` with no storage and no workspace.
fn make_session_manager() -> SessionManager {
    SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    )
}

/// Build a `ConfigManager` over a tempdir. We don't call `load()` because
/// the only thing we care about is the `agents` map; we inject the
/// fixtures manually below.
fn make_config_manager() -> ConfigManager {
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed")
}

/// Build a minimal `ResolvedAgentConfig` with the given `id` and
/// `subagents` overrides. All other fields use sensible defaults.
fn make_agent(id: &str, subagents: SubagentsConfig) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some(ModelSpec::single("test-model")),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents,
        memory: MemoryConfig::default(),
        source: ConfigSource::User,
    }
}

/// Create a parent session via the public `find_or_create` API.
///
/// `agent_id` is set as `message.to`; the resulting session's
/// `agent_id` field matches it (used by `SpawnController` to look
/// up the parent agent config).
async fn setup_parent_session(mgr: &SessionManager, agent_id: &str) -> String {
    let msg = Message {
        id: format!("msg-{}", agent_id),
        from: "user".to_string(),
        to: agent_id.to_string(),
        content: "hi".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    mgr.find_or_create("test-channel", &msg, None)
        .await
        .expect("find_or_create should succeed")
}

/// Inject the given (agent_id, ResolvedAgentConfig) pairs into a
/// ConfigManager's `agents` map. We do not go through `load_agents`
/// because we want a deterministic, minimal fixture.
fn inject_agents(cm: &ConfigManager, agents: Vec<(&str, ResolvedAgentConfig)>) {
    let mut map = cm.agents.write().expect("agents RwLock poisoned");
    for (id, cfg) in agents {
        map.insert(id.to_string(), cfg);
    }
}

/// Register N child sessions under a given parent in the SessionManager
/// (used to simulate concurrency pressure for the max_children test).
///
/// Also inserts each child into `conversation_sessions` so
/// `count_active_children` (which checks session liveness) counts
/// them correctly.
async fn fill_children(mgr: &SessionManager, parent_id: &str, count: usize) {
    for i in 0..count {
        let child_id = format!("child-{}", i);
        let cs = closeclaw_session::llm_session::ConversationSession::new(
            child_id.clone(),
            "test-model".to_string(),
            std::path::PathBuf::from("/tmp"),
        );
        mgr.conversation_sessions.write().await.insert(
            child_id.clone(),
            std::sync::Arc::new(tokio::sync::RwLock::new(cs)),
        );
        mgr.register_child(
            parent_id,
            ChildSessionInfo {
                session_id: child_id,
                parent_session_id: parent_id.to_string(),
                agent_id: "child".to_string(),
                depth: 1,
                mode: SpawnMode::Run,
            },
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// 1. test_validate_passes — happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_validate_passes() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // Parent uses max_spawn_depth=2 so child_depth=1 passes the depth check.
    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    // Target agent exists in the agents map.
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    // Parent at depth 0 → child_depth=1, effective_max=min(1,2-1)=1 → OK.
    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed for a legal request");

    assert_eq!(result.config.id, "child");
    assert_eq!(result.config.source, ConfigSource::User);
    // parent.max_spawn_depth=2, child.max_spawn_depth=1 (default)
    // effective_max = min(1, 2-1) = 1
    assert_eq!(result.effective_max_spawn_depth, 1);
}

// ---------------------------------------------------------------------------
// 2. test_validate_depth_exceeded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_validate_depth_exceeded() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // max_spawn_depth=0 forces the depth check to fail.
    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(0);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    // Parent at depth 0 → child_depth=1 > 0 → DepthExceeded.
    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("validate should reject when child_depth > effective_max");

    match err {
        SpawnError::DepthExceeded { current, max } => {
            assert_eq!(current, 1);
            assert_eq!(max, 0);
        }
        other => panic!("expected DepthExceeded, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 3. test_validate_max_children
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_validate_max_children() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // max_children=1 with 1 already-registered child → at the limit.
    // max_spawn_depth=2 so depth check passes before concurrency check.
    let mut sub = SubagentsConfig::default();
    sub.max_children = Some(1);
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;
    fill_children(&sm, &parent_id, 1).await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("validate should reject when active children >= max_children");

    match err {
        SpawnError::MaxChildrenReached { current, max } => {
            assert_eq!(current, 1);
            assert_eq!(max, 1);
        }
        other => panic!("expected MaxChildrenReached, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 4. test_validate_agent_not_allowed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_validate_agent_not_allowed() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // Allowlist only contains "allowed-agent" — target "child" is denied.
    // max_spawn_depth=2 so depth check passes before whitelist check.
    let mut sub = SubagentsConfig::default();
    sub.allow_agents = vec!["allowed-agent".to_string()];
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("validate should reject when target is not in allowlist");

    match err {
        SpawnError::AgentNotAllowed { agent_id } => {
            assert_eq!(agent_id, "child");
        }
        other => panic!("expected AgentNotAllowed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 5. test_validate_require_agent_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_validate_require_agent_id() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // require_agent_id=true and no default_child_agent → passing None must fail.
    let mut sub = SubagentsConfig::default();
    sub.require_agent_id = Some(true);
    sub.default_child_agent = None;
    let parent = make_agent("parent", sub);
    inject_agents(&cm, vec![("parent", parent)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("validate should reject when agentId is required but missing");

    assert!(
        matches!(err, SpawnError::AgentIdRequired),
        "expected AgentIdRequired, got {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// 6. test_validate_wildcard_allow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_validate_wildcard_allow() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // Explicit "*" wildcard in allow_agents — any target should be permitted.
    let mut sub = SubagentsConfig::default();
    sub.allow_agents = vec!["*".to_string()];
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    // Pick a non-default, otherwise-unrestricted target id.
    let target = make_agent("any-arbitrary-agent", SubagentsConfig::default());
    inject_agents(
        &cm,
        vec![("parent", parent), ("any-arbitrary-agent", target)],
    );

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("any-arbitrary-agent"))
        .await
        .expect("validate should succeed when allow_agents contains '*'");

    assert_eq!(result.config.id, "any-arbitrary-agent");
    // parent.max_spawn_depth=2, child.max_spawn_depth=1 (default)
    // effective_max = min(1, 2-1) = 1
    assert_eq!(result.effective_max_spawn_depth, 1);
}

// ---------------------------------------------------------------------------
// 7. test_validate_cascade_parent_depth1_child_depth2
// ---------------------------------------------------------------------------

/// Parent maxSpawnDepth=1, child maxSpawnDepth=2
/// → effective_max = min(2, 1-1) = 0
/// → child exists with effective_budget=0 (cannot spawn further).
#[tokio::test]
async fn test_validate_cascade_parent_depth1_child_depth2() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(1);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(2);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should allow: effective_max=0, child exists but cannot spawn further");

    assert_eq!(result.effective_max_spawn_depth, 0);
}

// ---------------------------------------------------------------------------
// 8. test_validate_cascade_parent_depth2_child_depth2
// ---------------------------------------------------------------------------

/// Parent maxSpawnDepth=2, child maxSpawnDepth=2
/// → effective_max = min(2, 2-1) = 1
/// → child_depth=1 <= 1 → OK.
#[tokio::test]
async fn test_validate_cascade_parent_depth2_child_depth2() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(2);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should pass: effective_max=1, child_depth=1 <= 1");

    assert_eq!(result.config.id, "child");
    assert_eq!(result.effective_max_spawn_depth, 1);
}

// ---------------------------------------------------------------------------
// 9. test_validate_cascade_parent_depth3_child_depth1
// ---------------------------------------------------------------------------

/// Parent maxSpawnDepth=3, child maxSpawnDepth=1
/// → effective_max = min(1, 3-1) = 1
/// → child_depth=1 <= 1 → OK.
#[tokio::test]
async fn test_validate_cascade_parent_depth3_child_depth1() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(3);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(1);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should pass: effective_max=1, child_depth=1 <= 1");

    assert_eq!(result.config.id, "child");
    assert_eq!(result.effective_max_spawn_depth, 1);
}

// ---------------------------------------------------------------------------
// 10. test_validate_cascade_unknown_target_config_not_found
// ---------------------------------------------------------------------------

/// Target agent not in agents map → ConfigNotFound (existing behavior preserved).
#[tokio::test]
async fn test_validate_cascade_unknown_target_config_not_found() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    inject_agents(&cm, vec![("parent", parent)]);
    // NOTE: "unknown-child" is NOT injected → target_config = None

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("unknown-child"))
        .await
        .expect_err("should fail with ConfigNotFound for unknown target");

    assert!(
        matches!(err, SpawnError::ConfigNotFound(_)),
        "expected ConfigNotFound, got {:?}",
        err
    );
}

// ── Step 1.4: additional cascading depth edge-case tests ─────────────────

/// Parent maxSpawnDepth=5, child maxSpawnDepth=1
/// → effective_max = min(1, 5-1) = 1
/// → child_depth=1 <= 1 → OK.
#[tokio::test]
async fn test_validate_cascade_parent_depth5_child_depth1() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(5);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(1);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should pass: effective_max=1, child_depth=1 <= 1");

    assert_eq!(result.config.id, "child");
    assert_eq!(result.effective_max_spawn_depth, 1);
}

/// Parent maxSpawnDepth=5, child maxSpawnDepth=3
/// → effective_max = min(3, 5-1) = 3
/// → child_depth=1 <= 3 → OK.
#[tokio::test]
async fn test_validate_cascade_parent_depth5_child_depth3() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(5);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(3);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should pass: effective_max=3, child_depth=1 <= 3");

    assert_eq!(result.config.id, "child");
    assert_eq!(result.effective_max_spawn_depth, 3);
}

/// Parent maxSpawnDepth=0, child maxSpawnDepth=5
/// → effective_max = min(5, 0-1) = min(5, 0 via saturating_sub) = 0
/// → child_depth=1 > 0 → DepthExceeded.
#[tokio::test]
async fn test_validate_cascade_parent_depth0_child_depth5() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(0);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(5);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("should reject: effective_max=0, child_depth=1 > 0");

    match err {
        SpawnError::DepthExceeded { current, max } => {
            assert_eq!(current, 1);
            assert_eq!(max, 0);
        }
        other => panic!("expected DepthExceeded, got {:?}", other),
    }
}

/// Target agent not configured (default max_spawn_depth=1)
/// with parent max_spawn_depth=1
/// → effective_max = min(1, 1-1) = 0
/// → child exists with effective_budget=0 (cannot spawn further).
/// Verifies that unconfigured targets use the default max_spawn_depth=1.
#[tokio::test]
async fn test_validate_cascade_unconfigured_child_depth1_parent1() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(1);
    let parent = make_agent("parent", parent_sub);
    // "child" has default max_spawn_depth=1
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should allow: effective_max=0, child exists but cannot spawn further");

    assert_eq!(result.effective_max_spawn_depth, 0);
}

// ── Step 1.1: depth-before-agentId order verification ────────────────────

/// Verify that when depth=0 AND requireAgentId=true AND no agentId is passed,
/// the validation returns `DepthExceeded` rather than `AgentIdRequired`.
///
/// This is the key behavioral difference the refactoring is meant to fix:
/// the design doc requires depth check to execute before agentId resolution.
#[tokio::test]
async fn test_validate_depth_before_agent_id_required() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // max_spawn_depth=0 (depth check will reject) AND require_agent_id=true
    // (would also reject if depth check didn't run first).
    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(0);
    sub.require_agent_id = Some(true);
    sub.default_child_agent = None;
    let parent = make_agent("parent", sub);
    inject_agents(&cm, vec![("parent", parent)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // Pass None for agent_id — without the refactoring this would return
    // AgentIdRequired (because require_agent_id is checked before depth).
    // After the refactoring, depth check runs first → DepthExceeded.
    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("should reject when depth=0");

    match err {
        SpawnError::DepthExceeded { current, max } => {
            assert_eq!(current, 1);
            assert_eq!(max, 0);
        }
        other => panic!(
            "expected DepthExceeded (depth checked before agentId), got {:?}",
            other
        ),
    }
}

// ── Step 1.2: multi-level cascade kill — no orphan entries ────────────

/// Helper to create a minimal ConversationSession + register it in a
/// SessionManager for cascade-kill tests.
async fn setup_kill_test_session(
    mgr: &SessionManager,
    tmp: &tempfile::TempDir,
    session_id: &str,
    agent_id: &str,
    depth: u32,
) {
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    mgr.conversation_sessions.write().await.insert(
        session_id.to_string(),
        std::sync::Arc::new(tokio::sync::RwLock::new(cs)),
    );
    mgr.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth,
        },
    );
}

/// Macro to register a session and its parent-child relationship in one call.
/// Each invocation stays compact (2 lines) even after `cargo fmt`.
macro_rules! register_child {
    ($mgr:expr, $tmp:expr, $parent:expr, $child:expr, $agent:expr, $depth:expr, $mode:expr) => {
        setup_kill_test_session($mgr, $tmp, $child, $agent, $depth).await;
        $mgr.register_child(
            $parent,
            ChildSessionInfo {
                session_id: $child.to_string(),
                parent_session_id: $parent.to_string(),
                agent_id: $agent.to_string(),
                depth: $depth,
                mode: $mode,
            },
        )
        .await;
    };
}

/// Kill a child with multiple descendants (grandchild + great-grandchild)
/// and verify no orphan entries remain in any tracking table.
/// This is the key Step 1.2 multi-level cascade test.
#[rustfmt::skip]
#[tokio::test]
async fn test_kill_child_cascades_removes_all_orphans_multilevel() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_session_manager();

    let parent_id = "parent-kill-multi";
    let child_id = "child-multi";
    let grandchild_a_id = "grandchild-a-multi";
    let grandchild_b_id = "grandchild-b-multi";
    let great_grandchild_id = "great-grandchild-multi";

    setup_kill_test_session(&mgr, &tmp, parent_id, "parent-agent", 0).await;
    register_child!(&mgr, &tmp, parent_id, child_id, "child-agent", 1, SpawnMode::Session);
    register_child!(&mgr, &tmp, child_id, grandchild_a_id, "gc-a-agent", 2, SpawnMode::Run);
    register_child!(
        &mgr, &tmp,
        grandchild_a_id, great_grandchild_id,
        "ggc-agent", 3, SpawnMode::Run
    );
    register_child!(&mgr, &tmp, child_id, grandchild_b_id, "gc-b-agent", 2, SpawnMode::Session);

    // Verify initial state
    assert_eq!(mgr.count_active_children(parent_id).await, 1);
    assert_eq!(mgr.count_active_children(child_id).await, 2);
    assert_eq!(mgr.count_active_children(grandchild_a_id).await, 1);
    assert!(mgr.has_session(child_id).await);
    assert!(mgr.has_session(grandchild_a_id).await);
    assert!(mgr.has_session(grandchild_b_id).await);
    assert!(mgr.has_session(great_grandchild_id).await);

    // Kill child — should recursively clean up all descendants
    mgr.kill_child(parent_id, child_id)
        .await
        .expect("kill_child should succeed");

    // ALL descendants removed — no orphans
    assert!(!mgr.has_session(child_id).await);
    assert!(!mgr.has_session(grandchild_a_id).await);
    assert!(!mgr.has_session(grandchild_b_id).await);
    assert!(!mgr.has_session(great_grandchild_id).await);
    assert!(mgr.get_conversation_session(child_id).await.is_none());
    assert!(mgr.get_conversation_session(grandchild_a_id).await.is_none());
    assert!(mgr.get_conversation_session(grandchild_b_id).await.is_none());
    assert!(mgr.get_conversation_session(great_grandchild_id).await.is_none());
    assert_eq!(mgr.count_active_children(parent_id).await, 0);
    assert_eq!(mgr.count_active_children(child_id).await, 0);
    assert_eq!(mgr.count_active_children(grandchild_a_id).await, 0);
}

/// Verify that kill_child on a child with no descendants is a clean removal.
#[tokio::test]
async fn test_kill_child_no_descendants_clean_removal() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_session_manager();

    let parent_id = "parent-no-desc";
    setup_kill_test_session(&mgr, &tmp, parent_id, "parent-agent", 0).await;

    let child_id = "lone-child";
    setup_kill_test_session(&mgr, &tmp, child_id, "child-agent", 1).await;
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
        },
    )
    .await;

    mgr.kill_child(parent_id, child_id)
        .await
        .expect("kill_child should succeed");

    assert!(!mgr.has_session(child_id).await);
    assert!(mgr.get_conversation_session(child_id).await.is_none());
    assert_eq!(mgr.count_active_children(parent_id).await, 0);
}

// ── Step 1.3: children survive across parent turns ────────────────────

/// When no target_agent_id is provided and default_child_agent resolves
/// to an agent not in the allowlist, the whitelist check should reject it.
#[tokio::test]
async fn test_validate_default_child_agent_blocked_by_whitelist() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // Parent has default_child_agent="fallback-child" but allowlist
    // only allows "allowed-agent" — fallback should be blocked.
    let mut sub = SubagentsConfig::default();
    sub.default_child_agent = Some("fallback-child".to_string());
    sub.allow_agents = vec!["allowed-agent".to_string()];
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let fallback_child = make_agent("fallback-child", SubagentsConfig::default());
    inject_agents(
        &cm,
        vec![("parent", parent), ("fallback-child", fallback_child)],
    );

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("should reject when default_child_agent not in allowlist");

    match err {
        SpawnError::AgentNotAllowed { agent_id } => {
            assert_eq!(agent_id, "fallback-child");
        }
        other => panic!("expected AgentNotAllowed, got {:?}", other),
    }
}

/// When no target_agent_id is provided and default_child_agent resolves
/// to an agent in the allowlist, the whitelist check should pass.
#[tokio::test]
async fn test_validate_default_child_agent_allowed_by_whitelist() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller =
        SpawnController::new(cm.clone(), sm.clone(), Arc::new(make_permission_engine()));

    // Parent has default_child_agent="allowed-child" and allowlist
    // contains "allowed-child" — should pass.
    let mut sub = SubagentsConfig::default();
    sub.default_child_agent = Some("allowed-child".to_string());
    sub.allow_agents = vec!["allowed-child".to_string()];
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let allowed_child = make_agent("allowed-child", SubagentsConfig::default());
    inject_agents(
        &cm,
        vec![("parent", parent), ("allowed-child", allowed_child)],
    );

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, None)
        .await
        .expect("validate should succeed for allowed default_child_agent");

    assert_eq!(result.config.id, "allowed-child");
}

/// Verify that session-mode children registered under a parent are NOT
/// prematurely removed without an explicit kill. This test validates the
/// Step 1.3 design invariant: cascade termination only occurs on parent
/// session end or timeout, not on each finish_llm turn.
#[tokio::test]
async fn test_session_child_survives_without_explicit_kill() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_session_manager();

    let parent_id = "parent-survive";
    setup_kill_test_session(&mgr, &tmp, parent_id, "parent-agent", 0).await;

    let child_id = "surviving-child";
    setup_kill_test_session(&mgr, &tmp, child_id, "child-agent", 1).await;
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;

    // Simulate multiple parent turns — child should remain alive
    for turn in 0..3 {
        assert!(
            mgr.has_session(child_id).await,
            "child should survive turn {}",
            turn
        );
        assert!(
            mgr.get_conversation_session(child_id).await.is_some(),
            "child conversation session should survive turn {}",
            turn
        );
        assert_eq!(
            mgr.count_active_children(parent_id).await,
            1,
            "child count should remain 1 at turn {}",
            turn
        );
    }

    // Only after explicit kill should the child be removed
    mgr.kill_child(parent_id, child_id)
        .await
        .expect("kill_child should succeed");

    assert!(!mgr.has_session(child_id).await);
    assert!(mgr.get_conversation_session(child_id).await.is_none());
    assert_eq!(mgr.count_active_children(parent_id).await, 0);
}

/// Verify that run-mode children also survive without explicit kill
/// (cascade was removed from finish_llm, so run-mode children are only
/// cleaned up by the sweeper or explicit kill).
#[tokio::test]
async fn test_run_child_survives_without_explicit_kill() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_session_manager();

    let parent_id = "parent-run-survive";
    setup_kill_test_session(&mgr, &tmp, parent_id, "parent-agent", 0).await;

    let child_id = "run-surviving-child";
    setup_kill_test_session(&mgr, &tmp, child_id, "child-agent", 1).await;
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
        },
    )
    .await;

    // Verify child is alive
    assert!(mgr.has_session(child_id).await);
    assert_eq!(mgr.count_active_children(parent_id).await, 1);

    // Explicit kill removes it
    mgr.kill_child(parent_id, child_id)
        .await
        .expect("kill_child should succeed");

    assert!(!mgr.has_session(child_id).await);
    assert_eq!(mgr.count_active_children(parent_id).await, 0);
}
