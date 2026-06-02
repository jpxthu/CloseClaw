//! Unit tests for SpawnController::validate.
//!
//! Covers the 4 rejection scenarios + 2 success scenarios defined in
//! the plan (Step 1.8.A). Each test sets up a minimal `ConfigManager`
//! + `SessionManager` fixture and exercises the validation flow end-to-end.
//!
//! All tests use `#[tokio::test]` because `SpawnController::validate`
//! awaits on `SessionManager` methods.

use std::sync::Arc;

use crate::agent::config::SubagentsConfig;
use crate::agent::spawn::{SpawnController, SpawnError};
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::config::ConfigManager;
use crate::gateway::session_manager::{ChildSessionInfo, SpawnMode};
use crate::gateway::{DmScope, GatewayConfig, Message, SessionManager};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;

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
    }
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
        model: Some("test-model".to_string()),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents,
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
async fn fill_children(mgr: &SessionManager, parent_id: &str, count: usize) {
    for i in 0..count {
        mgr.register_child(
            parent_id,
            ChildSessionInfo {
                session_id: format!("child-{}", i),
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
    let controller = SpawnController::new(cm.clone(), sm.clone());

    // Parent uses default subagents config (allow_agents=["*"], max_spawn_depth=1,
    // max_children=5, require_agent_id=false).
    let parent = make_agent("parent", SubagentsConfig::default());
    // Target agent exists in the agents map.
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    // Parent at depth 0 → child_depth=1, max_spawn_depth=1 → OK.
    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed for a legal request");

    assert_eq!(result.id, "child");
    assert_eq!(result.source, ConfigSource::User);
}

// ---------------------------------------------------------------------------
// 2. test_validate_depth_exceeded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_validate_depth_exceeded() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(cm.clone(), sm.clone());

    // max_spawn_depth=0 forces the depth check to fail.
    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = 0;
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    // Parent at depth 0 → child_depth=1 > 0 → DepthExceeded.
    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("validate should reject when child_depth > max_spawn_depth");

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
    let controller = SpawnController::new(cm.clone(), sm.clone());

    // max_children=1 with 1 already-registered child → at the limit.
    let mut sub = SubagentsConfig::default();
    sub.max_children = 1;
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
    let controller = SpawnController::new(cm.clone(), sm.clone());

    // Allowlist only contains "allowed-agent" — target "child" is denied.
    let mut sub = SubagentsConfig::default();
    sub.allow_agents = vec!["allowed-agent".to_string()];
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
    let controller = SpawnController::new(cm.clone(), sm.clone());

    // require_agent_id=true and no default_child_agent → passing None must fail.
    let mut sub = SubagentsConfig::default();
    sub.require_agent_id = true;
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
    let controller = SpawnController::new(cm.clone(), sm.clone());

    // Explicit "*" wildcard in allow_agents — any target should be permitted.
    let mut sub = SubagentsConfig::default();
    sub.allow_agents = vec!["*".to_string()];
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

    assert_eq!(result.id, "any-arbitrary-agent");
}
