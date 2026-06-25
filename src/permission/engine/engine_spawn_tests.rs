use super::engine_eval::PermissionEngine;
use super::engine_spawn::SpawnPermissionError;
use super::engine_types::Effect;
use crate::agent::config::AgentPermissions;
use crate::permission::rules::RuleSetBuilder;
use std::collections::HashMap;

fn make_engine() -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset)
}

fn make_allowed_perms(agent_id: &str) -> AgentPermissions {
    let dims = [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ];
    let permissions = dims
        .iter()
        .map(|&dim| {
            (
                dim.to_string(),
                crate::agent::config::ActionPermission {
                    allowed: true,
                    limits: crate::agent::config::PermissionLimits::default(),
                },
            )
        })
        .collect();
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions,
        inherited_from: None,
    }
}

fn make_fully_denied_perms(agent_id: &str) -> AgentPermissions {
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    }
}

// -------------------------------------------------------------------------
// validate_and_inject_spawn tests (no caching)
// -------------------------------------------------------------------------

#[test]
fn test_validate_and_inject_spawn_success() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");

    let result = engine.validate_and_inject_spawn("child", &child, &parent, None, None, None);
    assert!(result.is_ok());
}

#[test]
fn test_validate_and_inject_spawn_fully_denied() {
    let engine = make_engine();
    let child = make_fully_denied_perms("child");
    let parent = make_allowed_perms("parent");

    let result = engine.validate_and_inject_spawn("child", &child, &parent, None, None, None);
    assert!(result.is_err());
    match result.unwrap_err() {
        SpawnPermissionError::FullyDenied {
            child_agent_id,
            parent_agent_id,
        } => {
            assert_eq!(child_agent_id, "child");
            assert_eq!(parent_agent_id, "parent");
        }
        other => panic!("expected FullyDenied, got {:?}", other),
    }
}

// -------------------------------------------------------------------------
// Three-way intersection tests (child ∩ parent ∩ user)
// -------------------------------------------------------------------------

#[test]
fn test_validate_and_inject_spawn_user_deny_overrides() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");
    let user = make_fully_denied_perms("user-1");

    let result = engine.validate_and_inject_spawn(
        "child",
        &child,
        &parent,
        Some(&user),
        Some("user-1"),
        None,
    );
    assert!(result.is_err());
    match result.unwrap_err() {
        SpawnPermissionError::FullyDeniedWithUser {
            child_agent_id,
            parent_agent_id,
            user_id,
        } => {
            assert_eq!(child_agent_id, "child");
            assert_eq!(parent_agent_id, "parent");
            assert_eq!(user_id, "user-1");
        }
        other => panic!("expected FullyDeniedWithUser, got {:?}", other),
    }
}

#[test]
fn test_validate_and_inject_spawn_user_partial_deny() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");

    // User denies exec but allows everything else
    let mut user_perms_map = HashMap::new();
    for dim in &[
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ] {
        user_perms_map.insert(
            dim.to_string(),
            crate::agent::config::ActionPermission {
                allowed: *dim != "exec",
                limits: crate::agent::config::PermissionLimits::default(),
            },
        );
    }
    let user = AgentPermissions {
        agent_id: "user-2".to_string(),
        permissions: user_perms_map,
        inherited_from: None,
    };

    let result = engine.validate_and_inject_spawn(
        "child",
        &child,
        &parent,
        Some(&user),
        Some("user-2"),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_validate_and_inject_spawn_user_allow_full() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");
    let user = make_allowed_perms("user-3");

    let result = engine.validate_and_inject_spawn(
        "child",
        &child,
        &parent,
        Some(&user),
        Some("user-3"),
        None,
    );
    assert!(result.is_ok());
}

// -------------------------------------------------------------------------
// Concurrent test
// -------------------------------------------------------------------------

#[test]
fn test_concurrent_spawn_and_evaluate() {
    use std::sync::Arc;
    use std::thread;

    use super::engine_types::{PermissionRequest, PermissionRequestBody};

    let engine = Arc::new(make_engine());
    let parent = Arc::new(make_allowed_perms("parent"));

    let mut handles = vec![];

    // Spawn multiple threads doing validate_and_inject_spawn
    for i in 0..10 {
        let engine = Arc::clone(&engine);
        let parent = Arc::clone(&parent);
        handles.push(thread::spawn(move || {
            let child = make_allowed_perms(&format!("child-{}", i));
            let result = engine.validate_and_inject_spawn(
                &format!("child-{}", i),
                &child,
                &parent,
                None,
                None,
                None,
            );
            assert!(result.is_ok());
        }));
    }

    // Spawn multiple threads doing evaluate
    for i in 0..10 {
        let engine = Arc::clone(&engine);
        handles.push(thread::spawn(move || {
            let body = PermissionRequestBody::CommandExec {
                agent: format!("eval-agent-{}", i),
                cmd: "ls".to_string(),
                args: vec![],
            };
            let req = PermissionRequest::Bare(body);
            let _resp = engine.evaluate(req, None);
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}
