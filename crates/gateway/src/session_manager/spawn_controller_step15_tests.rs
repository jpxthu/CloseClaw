#![allow(deprecated)] // default_child_agent is deprecated; tests verify backward-compatible config parsing

//! Step 1.5 unit tests — design-doc alignment verification.
//!
//! Covers the 4 test categories specified in the plan:
//!   1. AgentRegistry query path verification
//!   2. validate() step ordering (requireAgentId before agentId resolution)
//!   3. global_spawn_timeout default (Some(300))
//!   4. Crate归属 test (closeclaw_daemon::SpawnController)
//!
//! Also covers boundary values: depth=0, maxChildren=0,
//! requireAgentId + empty allowlist.

use std::sync::Arc;

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};
use closeclaw_config::agents::{ModelSpec, SubagentsConfig};
use closeclaw_config::ConfigManager;
use closeclaw_session::persistence::ReasoningLevel;

use crate::session_manager::spawn_controller::{SpawnController, SpawnError};
use crate::session_manager::{ChildSessionInfo, SpawnMode};
use crate::{GatewayConfig, Message, SessionManager};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::rules::RuleSetBuilder;

// ---------------------------------------------------------------------------
// Helpers (duplicated per project convention)
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_permission_engine() -> PermissionEngine {
    PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap())
}

fn make_session_manager() -> SessionManager {
    SessionManager::new(&test_config(), None, None, ReasoningLevel::default())
}

fn make_config_manager() -> ConfigManager {
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed")
}

fn make_controller(
    ar: &Arc<AgentRegistry>,
    cm: &Arc<ConfigManager>,
    sm: &Arc<SessionManager>,
) -> SpawnController {
    SpawnController::new(
        Arc::clone(ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    )
}

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
        hooks: Vec::new(),
        source: ConfigSource::User,
    }
}

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

fn inject_agents(ar: &AgentRegistry, cm: &ConfigManager, agents: Vec<(&str, ResolvedAgentConfig)>) {
    let mut map = cm.agents.write().expect("agents RwLock poisoned");
    let mut configs = Vec::new();
    for (id, cfg) in agents {
        map.insert(id.to_string(), cfg.clone());
        configs.push(cfg);
    }
    ar.populate(configs);
}

/// Register N child sessions under a given parent in the SessionManager.
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

// ═══════════════════════════════════════════════════════════════════════════
// 1. AgentRegistry query path tests
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that SpawnController queries agent config through AgentRegistry.
/// When the registry does not contain the target agent, ConfigNotFound is
/// returned — not a crash or fallback to ConfigManager.agents().
///
/// This test ensures the query path goes through AgentRegistry (not
/// directly through ConfigManager.agents()), because:
///   - Only parent agents are injected into the registry
///   - Target "unknown-agent" is not in the registry
///   - ConfigNotFound proves the registry was the lookup source
#[tokio::test]
async fn test_agent_registry_query_path_config_not_found() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    // Parent is in the registry with max_spawn_depth=2 (passes depth check).
    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    inject_agents(&ar, &cm, vec![("parent", parent)]);
    // NOTE: "unknown-agent" is NOT injected — AgentRegistry returns None.

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("unknown-agent"))
        .await
        .expect_err("should fail with ConfigNotFound when registry has no target");

    assert!(
        matches!(err, SpawnError::ConfigNotFound(ref id) if id == "unknown-agent"),
        "expected ConfigNotFound(\"unknown-agent\"), got {:?}",
        err
    );
}

/// Verify that AgentRegistry returning None for the parent agent still
/// allows validation (parent config resolution falls back to defaults).
/// This is the read_parent_config fallback path.
#[tokio::test]
async fn test_agent_registry_parent_not_found_uses_defaults() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    // Do NOT inject "parent" — AgentRegistry returns None for it.
    // read_parent_config falls back to defaults: max_children=5,
    // allow_agents=["*"], require_agent_id=false.
    let parent_id = setup_parent_session(&sm, "parent").await;

    // "child" IS injected, so target config resolution succeeds.
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("child", child)]);

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should succeed: parent not in registry → defaults used");

    assert_eq!(result.config.id, "child");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Step ordering tests (requireAgentId before agentId resolution)
// ═══════════════════════════════════════════════════════════════════════════

/// KEY TEST: requireAgentId=true, no agentId provided, empty allowlist.
/// The validation should return AgentIdRequired (step ③), NOT AgentNotAllowed
/// (step ⑥). This proves requireAgentId is checked before agentId resolution
/// and whitelist check — the design doc ordering is correct.
///
/// If the ordering were wrong (agentId resolution before requireAgentId),
/// the fallback to parent agent ID would trigger the whitelist check and
/// return AgentNotAllowed instead.
#[tokio::test]
async fn test_step_order_require_agent_id_before_whitelist() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    // require_agent_id=true, allow_agents is empty (no parent, no child).
    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.require_agent_id = Some(true);
    sub.allow_agents = vec![]; // empty — if whitelist ran, it would reject
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("should reject: requireAgentId=true + no agentId");

    // CRITICAL: must be AgentIdRequired, NOT AgentNotAllowed
    assert!(
        matches!(err, SpawnError::AgentIdRequired),
        "expected AgentIdRequired (step ③), but got {:?} — ordering bug!",
        err
    );
}

/// requireAgentId=true, agentId provided → should proceed past step ③.
/// The whitelist check (step ⑥) then runs on the resolved target.
/// If target is not in allowlist → AgentNotAllowed (NOT AgentIdRequired).
#[tokio::test]
async fn test_step_order_agent_id_provided_whitelist_rejects() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.require_agent_id = Some(true);
    sub.allow_agents = vec!["only-this".to_string()];
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("should reject: 'child' not in allowlist");

    // AgentId was provided, so step ③ passes; step ⑥ whitelist rejects.
    assert!(
        matches!(err, SpawnError::AgentNotAllowed { ref agent_id } if agent_id == "child"),
        "expected AgentNotAllowed(\"child\"), got {:?}",
        err
    );
}

/// requireAgentId=true, no agentId, parent IS in allowlist.
/// Should still return AgentIdRequired — the whitelist is not checked
/// until after agentId resolution, and requireAgentId short-circuits.
#[tokio::test]
async fn test_step_order_require_agent_id_short_circuits_whitelist() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.require_agent_id = Some(true);
    // Parent IS in allowlist — if whitelist ran, it would pass.
    // But requireAgentId runs first → AgentIdRequired.
    sub.allow_agents = vec!["parent".to_string()];
    let parent = make_agent("parent", sub);
    inject_agents(&ar, &cm, vec![("parent", parent)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("should reject: requireAgentId=true + no agentId");

    assert!(
        matches!(err, SpawnError::AgentIdRequired),
        "expected AgentIdRequired, got {:?}",
        err
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. global_spawn_timeout tests
// ═══════════════════════════════════════════════════════════════════════════

/// Target agent has no subagents.timeout configured.
/// After Step 1.3, global_spawn_timeout() returns Some(300),
/// so spawn_timeout must be Some(300), NOT None.
#[tokio::test]
async fn test_global_spawn_timeout_fallback_to_300() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    // child has no timeout configured (timeout=None)
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");

    // Step 1.3: global_spawn_timeout() returns Some(300)
    assert_eq!(
        result.spawn_timeout,
        Some(300),
        "spawn_timeout should fall back to global default of 300s"
    );
}

/// Target agent has subagents.timeout=120 configured.
/// Priority chain: target config takes precedence over global default.
#[tokio::test]
async fn test_spawn_timeout_priority_target_over_global() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    let mut child_sub = SubagentsConfig::default();
    child_sub.timeout = Some(120);
    let child = make_agent("child", child_sub);
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");

    // Target config timeout (120) takes precedence over global default (300).
    assert_eq!(result.spawn_timeout, Some(120));
}

/// Target agent has subagents.timeout=0.
/// 0 is a valid value; passthrough as Some(0).
#[tokio::test]
async fn test_spawn_timeout_zero_passthrough() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    let mut child_sub = SubagentsConfig::default();
    child_sub.timeout = Some(0);
    let child = make_agent("child", child_sub);
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");

    assert_eq!(result.spawn_timeout, Some(0));
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Crate归属 test (compile-time check)
// ═══════════════════════════════════════════════════════════════════════════

// NOTE: crate归属 test lives in closeclaw-daemon crate (closeclaw-gateway
// cannot depend on closeclaw-daemon due to circular dependency).
// See crates/daemon/src/spawn_controller归属_tests.rs

// ═══════════════════════════════════════════════════════════════════════════
// 5. Boundary value tests
// ═══════════════════════════════════════════════════════════════════════════

/// depth=0: parent with max_spawn_depth=0 cannot spawn any children.
#[tokio::test]
async fn test_boundary_depth_zero() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(0);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("should reject: depth=0 means no spawning allowed");

    match err {
        SpawnError::DepthExceeded { current, max } => {
            assert_eq!(current, 1);
            assert_eq!(max, 0);
        }
        other => panic!("expected DepthExceeded, got {:?}", other),
    }
}

/// maxChildren=0: parent with max_children=0 rejects immediately.
/// Even depth check passes (max_spawn_depth=2), the concurrency check fails.
#[tokio::test]
async fn test_boundary_max_children_zero() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.max_children = Some(0);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("should reject: max_children=0");

    match err {
        SpawnError::MaxChildrenReached { current, max } => {
            assert_eq!(current, 0);
            assert_eq!(max, 0);
        }
        other => panic!("expected MaxChildrenReached, got {:?}", other),
    }
}

/// requireAgentId + empty allowlist: should return AgentIdRequired,
/// not AgentNotAllowed. This is the boundary where both constraints
/// could trigger — the ordering determines which error is returned.
#[tokio::test]
async fn test_boundary_require_agent_id_empty_allowlist() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.require_agent_id = Some(true);
    sub.allow_agents = vec![]; // empty
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("should reject: requireAgentId + no agentId");

    // requireAgentId (step ③) fires before whitelist (step ⑥).
    assert!(
        matches!(err, SpawnError::AgentIdRequired),
        "expected AgentIdRequired (not AgentNotAllowed), got {:?}",
        err
    );
}

/// depth=1 parent spawning child with max_spawn_depth=1
/// → effective_max = min(1, 1-1) = 0
/// → child gets effective_budget=0 (cannot spawn further).
#[tokio::test]
async fn test_boundary_depth_one_effective_max_zero() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(1);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(1);
    let child = make_agent("child", child_sub);
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should allow: child exists but effective_max=0");

    assert_eq!(result.effective_max_spawn_depth, 0);
}

/// maxChildren=1 with exactly 1 child already present → MaxChildrenReached.
#[tokio::test]
async fn test_boundary_max_children_one_saturated() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&ar, &cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.max_children = Some(1);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;
    fill_children(&sm, &parent_id, 1).await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("should reject: 1 child present, max_children=1");

    match err {
        SpawnError::MaxChildrenReached { current, max } => {
            assert_eq!(current, 1);
            assert_eq!(max, 1);
        }
        other => panic!("expected MaxChildrenReached, got {:?}", other),
    }
}
