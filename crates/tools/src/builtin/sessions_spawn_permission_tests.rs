//! Permission validation tests for spawn-related permission logic.
//!
//! Covers:
//! 1. Child agent all-deny → spawn returns PermissionDenied
//! 2. Child agent partial deny → spawn succeeds
//! 3. Parent effective permissions (no cache — config_manager provides base perms)
//! 4. SpawnError::PermissionDenied includes agent_id and reason
//! 5. Multi-generation spawn chain (depth=2)
//!
//! NOTE: Tests requiring `SpawnController` (a main-crate type) are marked
//! `#[ignore]` until they can be moved to integration tests or the main
//! crate test suite.

use std::collections::HashMap;

use closeclaw_common::agent_config::{ActionPermission, AgentPermissions, PermissionLimits};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_spawn::SpawnPermissionError;
use closeclaw_permission::rules::RuleSetBuilder;

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

// ===========================================================================
// Tests requiring SpawnController (main-crate type)
// ===========================================================================

/// Test: Fallback to config_manager's external permissions.json.
#[tokio::test]
#[ignore = "requires SpawnController from main crate"]
async fn test_external_permissions_used() {
    // Requires SpawnController which lives in the main crate.
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
#[ignore = "requires SpawnController from main crate"]
async fn test_fully_denied_silent_return_no_session_created() {
    // Requires SpawnController which lives in the main crate.
}
