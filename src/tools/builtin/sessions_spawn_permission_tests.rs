//! Permission validation tests for `SessionsSpawnTool`.
//!
//! Covers:
//! 1. Child agent all-deny → spawn returns PermissionDenied
//! 2. Child agent partial deny → spawn succeeds
//! 3. Parent effective permissions (no cache — config_manager provides base perms)
//! 4. SpawnError::PermissionDenied includes agent_id and reason
//! 5. Multi-generation spawn chain (depth=2)

use std::collections::HashMap;

use crate::agent::config::{ActionPermission, AgentPermissions, PermissionLimits};
use crate::agent::spawn::SpawnController;
use crate::config::ConfigManager;
use crate::gateway::session_manager::SessionManager;
use crate::gateway::{DmScope, GatewayConfig, Session};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_spawn::SpawnPermissionError;
use closeclaw_permission::rules::RuleSetBuilder;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `PermissionEngine` with an empty RuleSet.
fn make_permission_engine() -> PermissionEngine {
    PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap())
}

/// Create an `AgentPermissions` with all seven dimensions denied.
fn make_all_deny(agent_id: &str) -> AgentPermissions {
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    }
}

/// Create an `AgentPermissions` where the specified dimensions are denied
/// and the rest are allowed.
fn make_partial_deny(agent_id: &str, denied_dims: &[&str]) -> AgentPermissions {
    let all_dims = [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ];
    let mut permissions = HashMap::new();
    for &dim in &all_dims {
        permissions.insert(
            dim.to_string(),
            ActionPermission {
                allowed: !denied_dims.contains(&dim),
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

/// Create an `AgentPermissions` with all seven dimensions allowed.
fn make_all_allow(agent_id: &str) -> AgentPermissions {
    make_partial_deny(agent_id, &[])
}

// ===========================================================================
// Permission validation tests
// ===========================================================================

/// Test 1: Child agent all seven dimensions denied → spawn returns PermissionDenied.
#[tokio::test]
async fn test_child_all_deny_spawn_returns_permission_denied() {
    let engine = make_permission_engine();
    let child = make_all_deny("child-all-deny");
    let parent = make_all_allow("parent-agent");

    let result =
        engine.validate_and_inject_spawn("child-all-deny", &child, &parent, None, None, None);

    match result {
        Err(SpawnPermissionError::FullyDenied {
            child_agent_id,
            parent_agent_id,
        }) => {
            assert_eq!(child_agent_id, "child-all-deny");
            assert_eq!(parent_agent_id, "parent-agent");
        }
        Err(SpawnPermissionError::FullyDeniedWithUser { .. }) => {
            panic!("expected FullyDenied, not FullyDeniedWithUser");
        }
        other => panic!(
            "expected FullyDenied error for all-deny child, got: {:?}",
            other
        ),
    }
}

/// Test 2: Child agent partial deny → spawn succeeds.
#[tokio::test]
async fn test_child_partial_deny_spawn_success() {
    let engine = make_permission_engine();
    let child = make_partial_deny("child-partial", &["exec", "file_write"]);
    let parent = make_all_allow("parent-agent");

    let result =
        engine.validate_and_inject_spawn("child-partial", &child, &parent, None, None, None);
    assert!(
        result.is_ok(),
        "spawn should succeed for partial-deny child"
    );
}

/// Test 3: SpawnError::PermissionDenied includes agent_id and reason.
#[test]
fn test_spawn_error_permission_denied_includes_agent_id() {
    let err = SpawnPermissionError::FullyDenied {
        child_agent_id: "bad-agent-42".to_string(),
        parent_agent_id: "parent-agent".to_string(),
    };

    let msg = format!("{}", err);
    assert!(
        msg.contains("bad-agent-42"),
        "error message should contain child agent_id, got: {}",
        msg
    );
    assert!(
        msg.contains("parent-agent"),
        "error message should contain parent agent_id, got: {}",
        msg
    );
    assert!(
        msg.contains("denied"),
        "error message should mention 'denied', got: {}",
        msg
    );
}

/// Test 4: Multi-generation spawn chain (depth=2) — intersection logic.
#[test]
fn test_multi_generation_spawn_chain() {
    let engine = make_permission_engine();

    // Root agent: deny exec, allow rest.
    let root = make_partial_deny("root-agent", &["exec"]);

    // Child agent: all-allow (but inherits exec=deny from root).
    let child = make_all_allow("child-agent");

    // Spawn child under root.
    engine
        .validate_and_inject_spawn("child-agent", &child, &root, None, None, None)
        .expect("spawn of child under root should succeed");

    // Compute child's effective permissions (no cache — compute manually)
    let child_effective = child.intersect(&root);
    assert!(
        !child_effective.permissions.get("exec").unwrap().allowed,
        "child effective should have exec=deny (inherited from root)"
    );
    assert!(
        child_effective
            .permissions
            .get("file_read")
            .unwrap()
            .allowed,
        "child effective should have file_read=allow"
    );

    // Grandchild agent: all-allow (inherits from child's effective).
    let grandchild = make_all_allow("grandchild-agent");

    // Spawn grandchild under child.
    engine
        .validate_and_inject_spawn(
            "grandchild-agent",
            &grandchild,
            &child_effective,
            None,
            None,
            None,
        )
        .expect("spawn of grandchild under child should succeed");

    // Compute grandchild's effective permissions
    let grandchild_effective = grandchild.intersect(&child_effective);
    assert!(
        !grandchild_effective
            .permissions
            .get("exec")
            .unwrap()
            .allowed,
        "grandchild effective should have exec=deny (inherited chain root→child)"
    );
    assert!(
        grandchild_effective
            .permissions
            .get("file_read")
            .unwrap()
            .allowed,
        "grandchild effective should have file_read=allow"
    );
}

/// Test: Fallback to config_manager's external permissions.json.
#[tokio::test]
async fn test_external_permissions_used() {
    let tmp = TempDir::new().unwrap();

    let cm = Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    );
    {
        let mut ext_perms = cm.agent_permissions.write().unwrap();
        ext_perms.insert(
            "fallback-agent".to_string(),
            make_partial_deny("fallback-agent", &["exec"]),
        );
    }

    let config = crate::config::agents::ResolvedAgentConfig {
        id: "fallback-agent".to_string(),
        name: "fallback-agent".to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: crate::agent::config::SubagentsConfig::default(),
        memory: None,
        source: crate::config::agents::ConfigSource::Merged,
    };

    let gw_config = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &gw_config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    {
        let mut sessions = sm.sessions.write().await;
        sessions.insert(
            "parent-sess-2".to_string(),
            Session {
                id: "parent-sess-2".to_string(),
                agent_id: "parent-agent-2".to_string(),
                channel: "feishu".to_string(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    let pe = Arc::new(make_permission_engine());
    let controller = Arc::new(SpawnController::new(cm.clone(), sm.clone(), pe.clone()));

    // Inject both parent and child agents into ConfigManager so
    // SpawnController::validate() can resolve configs and pass all checks.
    {
        let mut agents = cm.agents.write().unwrap();
        agents.insert(
            "parent-agent-2".to_string(),
            crate::config::agents::ResolvedAgentConfig {
                id: "parent-agent-2".to_string(),
                name: "parent-agent-2".to_string(),
                parent_id: None,
                model: Some("test-model".to_string()),
                workspace: None,
                agent_dir: None,
                bootstrap_mode: BootstrapMode::Full,
                skills: vec![],
                tools: vec![],
                disallowed_tools: vec![],
                subagents: {
                    let mut sub = crate::agent::config::SubagentsConfig::default();
                    sub.max_spawn_depth = 2;
                    sub.allow_agents = vec!["fallback-agent".to_string()];
                    sub
                },
                memory: None,
                source: crate::config::agents::ConfigSource::User,
            },
        );
        agents.insert("fallback-agent".to_string(), config.clone());
    }

    let result = controller
        .validate("parent-sess-2", Some("fallback-agent"))
        .await;
    assert!(
        result.is_ok(),
        "validate should succeed with external permissions, got: {:?}",
        result
    );
}

// ===========================================================================
// Multi-layer recursive permission intersection
// ===========================================================================

/// Test: Three-layer spawn chain where each level introduces different
/// permission restrictions.
#[test]
fn test_recursive_permission_intersection_three_layers() {
    let engine = make_permission_engine();

    // depth=0: root denies exec + file_write, allows rest.
    let root = make_partial_deny("root-agent", &["exec", "file_write"]);

    // depth=1: child denies network + file_read, allows rest.
    let child = make_partial_deny("child-agent", &["network", "file_read"]);

    // Spawn child under root: effective = child ∩ root.
    engine
        .validate_and_inject_spawn("child-agent", &child, &root, None, None, None)
        .expect("spawn of child-agent should succeed");

    let child_effective = child.intersect(&root);

    // Child effective: exec=deny (root), file_read=deny (child),
    // file_write=deny (root), network=deny (child),
    // spawn=allow, tool_call=allow, config_write=allow.
    assert!(!child_effective.permissions.get("exec").unwrap().allowed);
    assert!(
        !child_effective
            .permissions
            .get("file_read")
            .unwrap()
            .allowed
    );
    assert!(
        !child_effective
            .permissions
            .get("file_write")
            .unwrap()
            .allowed
    );
    assert!(!child_effective.permissions.get("network").unwrap().allowed);
    assert!(child_effective.permissions.get("spawn").unwrap().allowed);
    assert!(
        child_effective
            .permissions
            .get("tool_call")
            .unwrap()
            .allowed
    );
    assert!(
        child_effective
            .permissions
            .get("config_write")
            .unwrap()
            .allowed
    );

    // depth=2: grandchild denies spawn, allows rest.
    let grandchild = make_partial_deny("grandchild-agent", &["spawn"]);

    // Spawn grandchild under child: effective = grandchild ∩ child_effective.
    engine
        .validate_and_inject_spawn(
            "grandchild-agent",
            &grandchild,
            &child_effective,
            None,
            None,
            None,
        )
        .expect("spawn of grandchild-agent should succeed");

    let gc_effective = grandchild.intersect(&child_effective);

    // Grandchild effective: all denied dims from every level accumulate.
    assert!(
        !gc_effective.permissions.get("exec").unwrap().allowed,
        "exec should be denied (from root)"
    );
    assert!(
        !gc_effective.permissions.get("file_read").unwrap().allowed,
        "file_read should be denied (from child)"
    );
    assert!(
        !gc_effective.permissions.get("file_write").unwrap().allowed,
        "file_write should be denied (from root)"
    );
    assert!(
        !gc_effective.permissions.get("network").unwrap().allowed,
        "network should be denied (from child)"
    );
    assert!(
        !gc_effective.permissions.get("spawn").unwrap().allowed,
        "spawn should be denied (from grandchild)"
    );
    assert!(
        gc_effective.permissions.get("tool_call").unwrap().allowed,
        "tool_call should be allowed (no restriction in chain)"
    );
    assert!(
        gc_effective
            .permissions
            .get("config_write")
            .unwrap()
            .allowed,
        "config_write should be allowed (no restriction in chain)"
    );
}

// ===========================================================================
// FullyDenied silent return through SessionsSpawnTool
// ===========================================================================

/// Test: When a child agent is fully denied, validate_spawn_permissions
/// returns SpawnError::PermissionDenied.
#[tokio::test]
async fn test_fully_denied_silent_return_no_session_created() {
    let tmp = TempDir::new().unwrap();

    let cm = Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    );
    cm.agent_permissions
        .write()
        .unwrap()
        .insert("denied-child".to_string(), make_all_deny("denied-child"));
    cm.agent_permissions.write().unwrap().insert(
        "parent-agent-deny".to_string(),
        make_all_allow("parent-agent-deny"),
    );

    let config = crate::config::agents::ResolvedAgentConfig {
        id: "denied-child".to_string(),
        name: "denied-child".to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: crate::agent::config::SubagentsConfig::default(),
        memory: None,
        source: crate::config::agents::ConfigSource::Merged,
    };

    let gw_config = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &gw_config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    {
        let mut sessions = sm.sessions.write().await;
        sessions.insert(
            "parent-sess-deny".to_string(),
            Session {
                id: "parent-sess-deny".to_string(),
                agent_id: "parent-agent-deny".to_string(),
                channel: "feishu".to_string(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    let pe = Arc::new(make_permission_engine());
    let controller = Arc::new(SpawnController::new(cm.clone(), sm.clone(), pe.clone()));

    // Inject both parent and child agents into ConfigManager.
    {
        let mut agents = cm.agents.write().unwrap();
        agents.insert(
            "parent-agent-deny".to_string(),
            crate::config::agents::ResolvedAgentConfig {
                id: "parent-agent-deny".to_string(),
                name: "parent-agent-deny".to_string(),
                parent_id: None,
                model: Some("test-model".to_string()),
                workspace: None,
                agent_dir: None,
                bootstrap_mode: BootstrapMode::Full,
                skills: vec![],
                tools: vec![],
                disallowed_tools: vec![],
                subagents: {
                    let mut sub = crate::agent::config::SubagentsConfig::default();
                    sub.max_spawn_depth = 2;
                    sub.allow_agents = vec!["denied-child".to_string()];
                    sub
                },
                memory: None,
                source: crate::config::agents::ConfigSource::User,
            },
        );
        agents.insert("denied-child".to_string(), config.clone());
    }

    let result = controller
        .validate("parent-sess-deny", Some("denied-child"))
        .await;

    assert!(
        result.is_err(),
        "validate should fail for fully-denied child"
    );
    match result.unwrap_err() {
        crate::agent::spawn::SpawnError::PermissionDenied { agent_id, reason } => {
            assert_eq!(agent_id, "denied-child");
            assert!(
                reason.contains("denied"),
                "reason should mention denied, got: {}",
                reason
            );
        }
        other => panic!("expected PermissionDenied, got: {:?}", other),
    }
}
