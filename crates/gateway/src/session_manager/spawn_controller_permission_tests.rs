//! Tests for SpawnController::validate() permission check (Step 1.4).
//!
//! Verifies that `validate()` delegates permission validation to the
//! PermissionEngine and returns `SpawnError::PermissionDenied` when the
//! child agent's permissions are fully denied after intersection with
//! the parent agent's effective permissions.

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{
    ActionPermission, AgentPermissions, ModelSpec, PermissionLimits, SubagentsConfig,
};
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};
use closeclaw_config::ConfigManager;
use closeclaw_session::persistence::ReasoningLevel;

use crate::session_manager::spawn_controller::{SpawnController, SpawnError};
use crate::{GatewayConfig, Message, SessionManager};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::rules::RuleSetBuilder;

// ---------------------------------------------------------------------------
// Helpers (duplicated from spawn_controller_tests.rs to keep modules self-contained)
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

/// Create an `AgentPermissions` with the given allow/deny per dimension.
#[allow(dead_code)]
fn make_perms(agent_id: &str, allowed_dims: &[&str]) -> AgentPermissions {
    let dimensions = [
        "command",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ];
    let mut permissions = HashMap::with_capacity(dimensions.len());
    for &dim in &dimensions {
        permissions.insert(
            dim.to_string(),
            ActionPermission {
                allowed: allowed_dims.contains(&dim),
                limits: PermissionLimits::default(),
            },
        );
    }
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions,
        inherited_from: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When the child agent has all permissions denied, `validate()` must
/// return `SpawnError::PermissionDenied` because the intersection with
/// the parent's permissions produces a fully-denied result.
#[tokio::test]
async fn test_validate_permission_denied_child_fully_denied() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    // Parent: all permissions allowed; depth budget allows child creation.
    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    // Child: all permissions denied.
    let child = make_agent("child", SubagentsConfig::default());

    let mut agents = HashMap::new();
    agents.insert("parent".to_string(), parent);
    agents.insert("child".to_string(), child);
    ar.populate(agents.values().cloned().collect());
    cm.restore_agents(agents);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("validate should reject when child permissions are fully denied");

    match err {
        SpawnError::PermissionDenied { agent_id, reason } => {
            assert_eq!(agent_id, "child");
            assert!(
                reason.contains("denied"),
                "reason should mention denial, got: {reason}"
            );
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

/// When the child has some permissions and the parent denies all of them,
/// the intersection is fully denied and `validate()` returns
/// `SpawnError::PermissionDenied`.
#[tokio::test]
async fn test_validate_permission_denied_parent_denies_all() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    let child = make_agent("child", SubagentsConfig::default());

    // Parent has everything denied; child has everything allowed.
    // Intersection: child ∩ parent = all denied.

    let mut agents = HashMap::new();
    agents.insert("parent".to_string(), parent);
    agents.insert("child".to_string(), child);
    ar.populate(agents.values().cloned().collect());
    cm.restore_agents(agents);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let err = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect_err("validate should reject when parent denies all permissions");

    match err {
        SpawnError::PermissionDenied { agent_id, reason } => {
            assert_eq!(agent_id, "child");
            assert!(
                reason.contains("denied"),
                "reason should mention denial, got: {reason}"
            );
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

/// When both parent and child have at least one permission dimension
/// allowed in common, the intersection is NOT fully denied and
/// `validate()` should proceed past the permission check.
#[tokio::test]
async fn test_validate_permission_allowed_partial_overlap() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    let child = make_agent("child", SubagentsConfig::default());

    // Parent allows exec only; child allows exec + file_read.
    // Intersection: exec=allow (both allow), everything else=deny.
    // Not fully denied because exec is allowed.

    let mut agents = HashMap::new();
    agents.insert("parent".to_string(), parent);
    agents.insert("child".to_string(), child);
    ar.populate(agents.values().cloned().collect());
    cm.restore_agents(agents);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // Should succeed because the intersection is not fully denied.
    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed when permissions partially overlap");

    assert_eq!(result.config.id, "child");
}

/// When neither parent nor child has any permissions configured,
/// `validate()` should proceed without error (no permissions to check).
#[tokio::test]
async fn test_validate_no_permissions_configured() {
    let ar = Arc::new(AgentRegistry::new());
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = SpawnController::new(
        Arc::clone(&ar),
        cm.clone(),
        sm.clone(),
        Arc::new(tokio::sync::RwLock::new(make_permission_engine())),
    );

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    let child = make_agent("child", SubagentsConfig::default());

    // Inject agents but NO permissions.
    let mut agents = HashMap::new();
    agents.insert("parent".to_string(), parent);
    agents.insert("child".to_string(), child);
    ar.populate(agents.values().cloned().collect());
    cm.restore_agents(agents);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed when no permissions are configured");

    assert_eq!(result.config.id, "child");
}
