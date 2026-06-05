//! Permission validation tests for `SessionsSpawnTool` (Step 1.4 of issue #873).
//!
//! Covers:
//! 1. Child agent all-deny → spawn returns PermissionDenied
//! 2. Child agent partial deny → spawn succeeds, effective permissions cached
//! 3. Parent effective permissions from cache hit (spawn chain simulation)
//! 4. SpawnError::PermissionDenied includes agent_id and reason
//! 5. Runtime permission enforcement via check_agent_effective_permissions
//! 6. Multi-generation spawn chain (depth=2)

use std::collections::HashMap;

use crate::agent::config::{ActionPermission, AgentPermissions, PermissionLimits};
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_spawn::SpawnPermissionError;
use crate::permission::engine::engine_types::PermissionRequestBody;
use crate::permission::rules::RuleSetBuilder;

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
///
/// When the intersection of child and parent permissions is fully denied,
/// `validate_and_inject_spawn` returns `Err(SpawnPermissionError::FullyDenied)`,
/// and `sessions_spawn` surfaces this as `ToolCallError::ExecutionFailed` with
/// "permission denied" in the message.
#[tokio::test]
async fn test_child_all_deny_spawn_returns_permission_denied() {
    let engine = make_permission_engine();
    let child = make_all_deny("child-all-deny");
    let parent = make_all_allow("parent-agent");

    // Simulate the permission validation that sessions_spawn performs.
    // Child all-deny × parent all-allow = fully denied.
    let result = engine.validate_and_inject_spawn("child-all-deny", &child, &parent);

    match result {
        Err(SpawnPermissionError::FullyDenied {
            child_agent_id,
            parent_agent_id,
        }) => {
            assert_eq!(child_agent_id, "child-all-deny");
            assert_eq!(parent_agent_id, "parent-agent");
        }
        other => panic!(
            "expected FullyDenied error for all-deny child, got: {:?}",
            other
        ),
    }

    // Verify the cache does NOT contain the child (was rejected).
    assert!(
        engine
            .get_agent_effective_permissions("child-all-deny")
            .is_none(),
        "all-deny child should not be cached after rejection"
    );
}

/// Test 2: Child agent partial deny → spawn succeeds, effective permissions
/// injected into cache.
///
/// Child denies exec + file_write, allows the rest. Parent is all-allow.
/// After successful spawn, the permission cache should contain the child
/// with exec=deny, file_write=deny, file_read=allow.
#[tokio::test]
async fn test_child_partial_deny_spawn_success_injects_cache() {
    let engine = make_permission_engine();
    let child = make_partial_deny("child-partial", &["exec", "file_write"]);
    let parent = make_all_allow("parent-agent");

    // Spawn should succeed (child is not fully denied).
    let result = engine.validate_and_inject_spawn("child-partial", &child, &parent);
    assert!(
        result.is_ok(),
        "spawn should succeed for partial-deny child"
    );

    // Verify the cache contains the child.
    let cached = engine
        .get_agent_effective_permissions("child-partial")
        .expect("child should be in cache after successful spawn");

    // Verify exec = deny.
    let exec_perm = cached
        .permissions
        .get("exec")
        .expect("exec dimension should exist");
    assert!(
        !exec_perm.allowed,
        "exec should be denied in cached effective permissions"
    );

    // Verify file_write = deny.
    let fw_perm = cached
        .permissions
        .get("file_write")
        .expect("file_write dimension should exist");
    assert!(
        !fw_perm.allowed,
        "file_write should be denied in cached effective permissions"
    );

    // Verify file_read = allow.
    let fr_perm = cached
        .permissions
        .get("file_read")
        .expect("file_read dimension should exist");
    assert!(
        fr_perm.allowed,
        "file_read should be allowed in cached effective permissions"
    );

    // Verify other allowed dimensions.
    for dim in &["network", "spawn", "tool_call", "config_write"] {
        let perm = cached
            .permissions
            .get(*dim)
            .unwrap_or_else(|| panic!("{dim} dimension should exist"));
        assert!(
            perm.allowed,
            "{dim} should be allowed in cached effective permissions"
        );
    }
}

/// Test 3: Parent effective permissions from cache hit (spawn chain simulation).
///
/// First spawn child A (injects A's effective permissions into cache).
/// Then use A's effective permissions as parent to spawn grandchild B.
/// Verify B's cached permissions = B original ∩ A effective.
#[tokio::test]
async fn test_parent_effective_permissions_from_cache() {
    let engine = make_permission_engine();

    // Parent: all-allow.
    let grandparent = make_all_allow("grandparent");

    // Child A: deny exec, allow rest.
    let child_a = make_partial_deny("child-a", &["exec"]);

    // Spawn A under grandparent.
    engine
        .validate_and_inject_spawn("child-a", &child_a, &grandparent)
        .expect("spawn of child-a should succeed");

    // Verify A's effective permissions are in cache.
    let a_effective = engine
        .get_agent_effective_permissions("child-a")
        .expect("child-a should be in cache");
    assert!(
        !a_effective.permissions.get("exec").unwrap().allowed,
        "child-a effective should have exec=deny"
    );

    // Grandchild B: deny file_write, allow rest.
    let child_b = make_partial_deny("child-b", &["file_write"]);

    // Spawn B under A (using A's effective permissions as parent).
    engine
        .validate_and_inject_spawn("child-b", &child_b, &a_effective)
        .expect("spawn of child-b under child-a should succeed");

    // Verify B's cached effective permissions.
    let b_effective = engine
        .get_agent_effective_permissions("child-b")
        .expect("child-b should be in cache");

    // B original ∩ A effective: exec=deny (from A), file_write=deny (from B),
    // all others=allow.
    assert!(
        !b_effective.permissions.get("exec").unwrap().allowed,
        "child-b effective should have exec=deny (inherited from child-a)"
    );
    assert!(
        !b_effective.permissions.get("file_write").unwrap().allowed,
        "child-b effective should have file_write=deny (own restriction)"
    );
    assert!(
        b_effective.permissions.get("file_read").unwrap().allowed,
        "child-b effective should have file_read=allow"
    );
    assert!(
        b_effective.permissions.get("network").unwrap().allowed,
        "child-b effective should have network=allow"
    );
    assert!(
        b_effective.permissions.get("spawn").unwrap().allowed,
        "child-b effective should have spawn=allow"
    );
    assert!(
        b_effective.permissions.get("tool_call").unwrap().allowed,
        "child-b effective should have tool_call=allow"
    );
    assert!(
        b_effective.permissions.get("config_write").unwrap().allowed,
        "child-b effective should have config_write=allow"
    );
}

/// Test 4: SpawnError::PermissionDenied includes agent_id and reason.
///
/// Verify the error message format contains the denied agent_id.
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

/// Test 5: Runtime permission enforcement via check_agent_effective_permissions.
///
/// After spawning a child with exec=deny and file_read=allow:
/// - Exec dimension check → Denied response
/// - file_read dimension check → None (allowed, continue normal evaluate)
#[tokio::test]
async fn test_runtime_permission_enforcement() {
    let engine = make_permission_engine();
    let child = make_partial_deny("child-rt", &["exec"]);
    let parent = make_all_allow("parent-agent");

    // Spawn child (injects into cache).
    engine
        .validate_and_inject_spawn("child-rt", &child, &parent)
        .expect("spawn should succeed");

    // Simulate an exec operation for child-rt.
    let exec_body = PermissionRequestBody::CommandExec {
        agent: "child-rt".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    };
    let result = engine.check_agent_effective_permissions("child-rt", &exec_body);
    assert!(
        result.is_some(),
        "exec dimension should be denied for child-rt"
    );
    match result.unwrap() {
        crate::permission::engine::engine_types::PermissionResponse::Denied { reason, .. } => {
            assert!(
                reason.contains("exec"),
                "denied reason should mention exec dimension, got: {}",
                reason
            );
        }
        other => panic!("expected Denied response, got: {:?}", other),
    }

    // Simulate a file_read operation for child-rt.
    let read_body = PermissionRequestBody::FileOp {
        agent: "child-rt".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "read".to_string(),
    };
    let result = engine.check_agent_effective_permissions("child-rt", &read_body);
    assert!(
        result.is_none(),
        "file_read dimension should be allowed (None = continue) for child-rt"
    );

    // Simulate a network operation for child-rt (allowed dimension).
    let net_body = PermissionRequestBody::NetOp {
        agent: "child-rt".to_string(),
        host: "example.com".to_string(),
        port: 443,
    };
    let result = engine.check_agent_effective_permissions("child-rt", &net_body);
    assert!(
        result.is_none(),
        "network dimension should be allowed for child-rt"
    );
}

/// Test 6: Multi-generation spawn chain (depth=2).
///
/// Root agent disables exec. Child spawns successfully (effective: exec=deny).
/// Grandchild spawns successfully, inherits exec=deny.
/// Verify grandchild's check_agent_effective_permissions returns Denied for exec.
#[tokio::test]
async fn test_multi_generation_spawn_chain() {
    let engine = make_permission_engine();

    // Root agent: deny exec, allow rest.
    let root = make_partial_deny("root-agent", &["exec"]);

    // Child agent: all-allow (but inherits exec=deny from root).
    let child = make_all_allow("child-agent");

    // Spawn child under root.
    engine
        .validate_and_inject_spawn("child-agent", &child, &root)
        .expect("spawn of child under root should succeed");

    // Verify child's effective permissions: exec=deny (inherited from root).
    let child_effective = engine
        .get_agent_effective_permissions("child-agent")
        .expect("child-agent should be in cache");
    assert!(
        !child_effective.permissions.get("exec").unwrap().allowed,
        "child-agent effective should have exec=deny (inherited from root)"
    );
    assert!(
        child_effective
            .permissions
            .get("file_read")
            .unwrap()
            .allowed,
        "child-agent effective should have file_read=allow"
    );

    // Grandchild agent: all-allow (inherits from child's effective).
    let grandchild = make_all_allow("grandchild-agent");

    // Spawn grandchild under child.
    engine
        .validate_and_inject_spawn("grandchild-agent", &grandchild, &child_effective)
        .expect("spawn of grandchild under child should succeed");

    // Verify grandchild's effective permissions: exec=deny (inherited chain).
    let grandchild_effective = engine
        .get_agent_effective_permissions("grandchild-agent")
        .expect("grandchild-agent should be in cache");
    assert!(
        !grandchild_effective
            .permissions
            .get("exec")
            .unwrap()
            .allowed,
        "grandchild-agent effective should have exec=deny (inherited chain root→child)"
    );
    assert!(
        grandchild_effective
            .permissions
            .get("file_read")
            .unwrap()
            .allowed,
        "grandchild-agent effective should have file_read=allow"
    );

    // Verify runtime enforcement on grandchild.
    let exec_body = PermissionRequestBody::CommandExec {
        agent: "grandchild-agent".to_string(),
        cmd: "rm".to_string(),
        args: vec!["-rf".to_string(), "/tmp/test".to_string()],
    };
    let result = engine.check_agent_effective_permissions("grandchild-agent", &exec_body);
    assert!(
        result.is_some(),
        "exec should be denied at runtime for grandchild-agent"
    );
    match result.unwrap() {
        crate::permission::engine::engine_types::PermissionResponse::Denied { reason, .. } => {
            assert!(
                reason.contains("exec"),
                "denied reason should mention exec dimension, got: {}",
                reason
            );
        }
        other => panic!(
            "expected Denied response for grandchild exec, got: {:?}",
            other
        ),
    }

    // Verify file_read is still allowed at runtime for grandchild.
    let read_body = PermissionRequestBody::FileOp {
        agent: "grandchild-agent".to_string(),
        path: "/tmp/data.txt".to_string(),
        op: "read".to_string(),
    };
    let result = engine.check_agent_effective_permissions("grandchild-agent", &read_body);
    assert!(
        result.is_none(),
        "file_read should be allowed at runtime for grandchild-agent"
    );
}
