#![allow(deprecated)] // default_child_agent is deprecated; tests verify backward-compatible config parsing

//! Step 1.4 unit tests — agentId fallback and spawn timeout.
//!
//! Covers the two gap-fixes defined in the plan:
//!   - Gap 1: agentId fallback chain (explicit → parent ID)
//!   - Gap 2: SubagentsConfig.timeout passthrough to SpawnValidationResult
//!
//! Helpers are duplicated from spawn_controller_tests.rs to keep this
//! module self-contained (project convention).

use std::sync::Arc;

use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};
use closeclaw_config::agents::{ModelSpec, SubagentsConfig};
use closeclaw_config::ConfigManager;
use closeclaw_session::persistence::ReasoningLevel;

use crate::session_manager::spawn_controller::{SpawnController, SpawnError};
use crate::{GatewayConfig, Message, SessionManager};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::rules::RuleSetBuilder;

// ---------------------------------------------------------------------------
// Helpers (duplicated from spawn_controller_tests.rs)
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

fn make_controller(cm: &Arc<ConfigManager>, sm: &Arc<SessionManager>) -> SpawnController {
    SpawnController::new(
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

fn inject_agents(cm: &ConfigManager, agents: Vec<(&str, ResolvedAgentConfig)>) {
    let mut map = cm.agents.write().expect("agents RwLock poisoned");
    for (id, cfg) in agents {
        map.insert(id.to_string(), cfg);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Gap 1: agentId fallback to parent ID
// ═══════════════════════════════════════════════════════════════════════════

/// No target_agent_id + requireAgentId=false
/// → fallback chain resolves to parent agent ID, spawn succeeds.
/// (default_child_agent is deprecated and ignored.)
#[tokio::test]
async fn test_validate_agent_id_fallback_to_parent() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.require_agent_id = Some(false);
    sub.default_child_agent = None;
    let parent = make_agent("parent", sub);
    inject_agents(&cm, vec![("parent", parent)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, None)
        .await
        .expect("should succeed: no agentId + requireAgentId=false → fallback to parent");

    assert_eq!(result.config.id, "parent");
    assert_eq!(result.effective_max_spawn_depth, 1);
}

/// Whitelist does not contain the parent agent ID → fallback resolves to
/// parent ID, but whitelist check rejects it.
/// (default_child_agent is deprecated and ignored.)
#[tokio::test]
async fn test_validate_agent_id_fallback_rejected_by_whitelist() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.require_agent_id = Some(false);
    sub.default_child_agent = None;
    sub.allow_agents = vec!["allowed-agent".to_string()];
    let parent = make_agent("parent", sub);
    inject_agents(&cm, vec![("parent", parent)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("should reject when fallback parent ID not in whitelist");

    match err {
        SpawnError::AgentNotAllowed { agent_id } => {
            assert_eq!(agent_id, "parent");
        }
        other => panic!("expected AgentNotAllowed, got {:?}", other),
    }
}

/// Explicit agentId provided → fallback chain is not triggered at all;
/// the explicit target is used directly.
#[tokio::test]
async fn test_validate_explicit_agent_id_no_fallback() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&cm, &sm);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    parent_sub.default_child_agent = Some("should-not-resolve".to_string());
    let parent = make_agent("parent", parent_sub);
    let child = make_agent("explicit-child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("explicit-child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("explicit-child"))
        .await
        .expect("should succeed with explicit agentId");

    assert_eq!(result.config.id, "explicit-child");
}

/// When default_child_agent is configured but no explicit agentId is given,
/// the fallback resolves to the parent agent ID (not default_child_agent).
/// default_child_agent is deprecated and ignored per design doc §④.
#[tokio::test]
async fn test_validate_default_child_agent_ignored_falls_back_to_parent() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    sub.default_child_agent = Some("my-default".to_string()); // deprecated, ignored
    sub.allow_agents = vec!["*".to_string()];
    let parent = make_agent("parent", sub);
    let default_child = make_agent("my-default", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("my-default", default_child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // No target_agent_id → falls back to parent agent ID ("parent"),
    // NOT to default_child_agent ("my-default").
    let result = controller
        .validate(&parent_id, None)
        .await
        .expect("should succeed using parent agent ID as default");

    assert_eq!(result.config.id, "parent");
}

// ═══════════════════════════════════════════════════════════════════════════
// Gap 2: spawn timeout passthrough
// ═══════════════════════════════════════════════════════════════════════════

/// Target agent config has subagents.timeout=60 → SpawnValidationResult
/// must include spawn_timeout=Some(60).
#[tokio::test]
async fn test_validate_spawn_timeout_configured() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let mut child_sub = SubagentsConfig::default();
    child_sub.timeout = Some(60);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");

    assert_eq!(result.spawn_timeout, Some(60));
}

/// Parent config has no subagents.timeout → spawn_timeout must be None.
#[tokio::test]
async fn test_validate_spawn_timeout_not_configured() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");

    assert_eq!(result.spawn_timeout, None);
}

/// Target agent config has subagents.timeout=0 → passthrough as Some(0).
/// Value 0 is syntactically valid; the enforcement layer decides whether
/// to treat it as immediate timeout or reject it at runtime.
#[tokio::test]
async fn test_validate_spawn_timeout_zero_passthrough() {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = make_controller(&cm, &sm);

    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let mut child_sub = SubagentsConfig::default();
    child_sub.timeout = Some(0);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");

    assert_eq!(result.spawn_timeout, Some(0));
}
