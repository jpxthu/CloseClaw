#![allow(deprecated)] // default_child_agent is deprecated; tests verify backward-compatible config parsing

//! Boundary and edge-case tests for SpawnController::validate (Step 1.6).
//!
//! Covers scenarios not in the main test file:
//! - empty allow_agents array behavior
//! - unconfigured parent fallback to defaults
//! - requireAgentId=false with no target
//! - default max_children concurrency boundary

use std::sync::Arc;

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};
use closeclaw_config::agents::{ModelSpec, SubagentsConfig};
use closeclaw_config::ConfigManager;
use closeclaw_session::persistence::ReasoningLevel;

use crate::session_manager::spawn_controller::{SpawnController, SpawnError};
use crate::session_manager::ChildSessionInfo;
use crate::{GatewayConfig, Message, SessionManager};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::rules::RuleSetBuilder;

// ---------------------------------------------------------------------------
// Helpers (duplicated to keep this file self-contained)
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

fn inject_agents(ar: &AgentRegistry, cm: &ConfigManager, agents: Vec<(&str, ResolvedAgentConfig)>) {
    let mut map = cm.agents.write().expect("agents RwLock poisoned");
    let mut configs = Vec::new();
    for (id, cfg) in agents {
        map.insert(id.to_string(), cfg.clone());
        configs.push(cfg);
    }
    ar.populate(configs);
}

async fn fill_children(mgr: &SessionManager, parent_id: &str, count: usize) {
    for i in 0..count {
        let child_id = format!("boundary-child-{}", i);
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
                agent_id: "boundary-child".to_string(),
                depth: 1,
                mode: crate::session_manager::SpawnMode::Run,
            },
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Empty `allow_agents` array with an explicit target should block it.
/// The whitelist check iterates the list and finds no match -> AgentNotAllowed.
#[tokio::test]
async fn test_validate_empty_allow_agents_blocks_target() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    let mut sub = SubagentsConfig::default();
    sub.allow_agents = vec![]; // explicitly empty
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("validate should reject when allow_agents is empty");

    match err {
        SpawnError::AgentNotAllowed { agent_id } => {
            assert_eq!(agent_id, "child");
        }
        other => panic!("expected AgentNotAllowed, got {:?}", other),
    }
}

/// Empty `allow_agents` array with no target should fail with
/// AgentNotAllowed — when no target_agent_id is provided, the parent
/// agent ID ("parent") is used as default, which is rejected by the
/// empty allowlist.
#[tokio::test]
async fn test_validate_empty_allow_agents_no_target_requires_id() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    let mut sub = SubagentsConfig::default();
    sub.allow_agents = vec![];
    sub.require_agent_id = Some(false);
    sub.default_child_agent = None;
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    inject_agents(&ar, &cm, vec![("parent", parent)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // No target_agent_id -> falls back to parent_agent_id ("parent").
    // Empty allowlist rejects "parent" -> AgentNotAllowed.
    let err = controller
        .validate(&parent_id, None)
        .await
        .expect_err("should fail when parent agent not in empty allowlist");

    match err {
        SpawnError::AgentNotAllowed { agent_id } => {
            assert_eq!(agent_id, "parent");
        }
        other => panic!("expected AgentNotAllowed, got {:?}", other),
    }
}

/// Parent not in agents map -> read_parent_config returns defaults:
/// max_children=5, allow_agents=["*"], require_agent_id=false.
/// A target with existing config should pass all checks.
#[tokio::test]
async fn test_validate_unparent_config_uses_defaults() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    // Only inject child, NOT parent -> parent falls back to defaults.
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("child", child)]);

    // Parent agent id = "unregistered-parent" -- not in agents map.
    let parent_id = setup_parent_session(&sm, "unregistered-parent").await;

    // Defaults: max_children=5 (concurrency OK), allow_agents=["*"] (whitelist OK),
    // require_agent_id=false (no id needed), max_spawn_depth=1 (from default).
    // effective_max = min(1, 1-1) = 0 -> child exists but cannot spawn further.
    let result = controller.validate(&parent_id, Some("child")).await.expect(
        "should pass: unregistered parent uses default config (max_children=5, allow=[wildcard])",
    );

    assert_eq!(result.config.id, "child");
    assert_eq!(result.effective_max_spawn_depth, 0);
}

/// requireAgentId=false, no target_agent_id -> parent-agent-id fallback
/// resolves to the parent agent itself. Validation succeeds because the
/// parent agent is in its own allowlist.
#[tokio::test]
async fn test_validate_require_agent_id_false_no_target_no_default() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    let mut sub = SubagentsConfig::default();
    sub.require_agent_id = Some(false);
    sub.default_child_agent = None;
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    inject_agents(&ar, &cm, vec![("parent", parent)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // No target_agent_id -> falls back to parent_agent_id ("parent").
    // Default allow_agents=["*"] -> whitelist passes.
    let result = controller
        .validate(&parent_id, None)
        .await
        .expect("should succeed: parent-agent-id fallback resolves to parent itself");

    assert_eq!(result.config.id, "parent");
}

/// Default max_children (5) allows up to 4 concurrent children.
/// Registering 4 children should succeed; 5th should fail.
#[tokio::test]
async fn test_validate_default_max_children_boundary() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    // Parent with default max_children=5 and max_spawn_depth=2.
    let mut sub = SubagentsConfig::default();
    sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&ar, &cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // Fill 4 children -- should be at limit (max_children=5, 4 active).
    fill_children(&sm, &parent_id, 4).await;

    // 5th child should fail (4 >= 5? no, 4 < 5, so it should pass).
    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("should pass: 4 active < max_children=5");
    assert_eq!(result.config.id, "child");

    // Add 5th child to reach the limit.
    fill_children(&sm, &parent_id, 1).await;

    // Now 5 active >= 5 -> should fail.
    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("should fail: 5 active >= max_children=5");

    match err {
        SpawnError::MaxChildrenReached { current, max } => {
            assert_eq!(current, 5);
            assert_eq!(max, 5);
        }
        other => panic!("expected MaxChildrenReached, got {:?}", other),
    }
}
