use super::engine_eval::PermissionEngine;
use super::engine_spawn::SpawnPermissionError;
use super::engine_types::{Effect, PermissionRequest, PermissionRequestBody, PermissionResponse};
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
// validate_and_inject_spawn tests
// -------------------------------------------------------------------------

#[test]
fn test_validate_and_inject_spawn_success() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");

    let result = engine.validate_and_inject_spawn("child", &child, &parent, None, None);
    assert!(result.is_ok());

    // Verify cache was populated
    let cache = engine.agent_permissions.read().unwrap();
    assert!(cache.contains_key("child"));
}

#[test]
fn test_validate_and_inject_spawn_fully_denied() {
    let engine = make_engine();
    let child = make_fully_denied_perms("child");
    let parent = make_allowed_perms("parent");

    let result = engine.validate_and_inject_spawn("child", &child, &parent, None, None);
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

    // Verify cache was NOT populated
    let cache = engine.agent_permissions.read().unwrap();
    assert!(!cache.contains_key("child"));
}

#[test]
fn test_validate_and_inject_spawn_idempotent() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");

    // First call succeeds
    engine
        .validate_and_inject_spawn("child", &child, &parent, None, None)
        .unwrap();
    // Second call succeeds (idempotent)
    let result = engine.validate_and_inject_spawn("child", &child, &parent, None, None);
    assert!(result.is_ok());
}

// -------------------------------------------------------------------------
// check_agent_effective_permissions tests
// -------------------------------------------------------------------------

#[test]
fn test_check_agent_effective_permissions_cache_miss() {
    let engine = make_engine();
    let body = PermissionRequestBody::CommandExec {
        agent: "unknown-agent".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    };

    let result = engine.check_agent_effective_permissions("unknown-agent", &body);
    assert!(result.is_none());
}

#[test]
fn test_check_agent_effective_permissions_deny() {
    let engine = make_engine();
    // Inject a fully denied agent into cache
    // Force inject by directly manipulating cache
    {
        let mut cache = engine.agent_permissions.write().unwrap();
        cache.insert(
            "denied-agent".to_string(),
            AgentPermissions {
                agent_id: "denied-agent".to_string(),
                permissions: HashMap::from([(
                    "exec".to_string(),
                    crate::agent::config::ActionPermission {
                        allowed: false,
                        limits: crate::agent::config::PermissionLimits::default(),
                    },
                )]),
                inherited_from: Some("parent".to_string()),
            },
        );
    }

    let body = PermissionRequestBody::CommandExec {
        agent: "denied-agent".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    };
    let result = engine.check_agent_effective_permissions("denied-agent", &body);
    assert!(result.is_some());
    match result.unwrap() {
        PermissionResponse::Denied { reason, .. } => {
            assert!(reason.contains("agent effective permission denied"));
        }
        other => panic!("expected Denied, got {:?}", other),
    }
}

#[test]
fn test_check_agent_effective_permissions_allow() {
    let engine = make_engine();
    // Inject an allowed agent into cache
    {
        let mut cache = engine.agent_permissions.write().unwrap();
        cache.insert(
            "allowed-agent".to_string(),
            AgentPermissions {
                agent_id: "allowed-agent".to_string(),
                permissions: HashMap::from([(
                    "exec".to_string(),
                    crate::agent::config::ActionPermission {
                        allowed: true,
                        limits: crate::agent::config::PermissionLimits::default(),
                    },
                )]),
                inherited_from: None,
            },
        );
    }

    let body = PermissionRequestBody::CommandExec {
        agent: "allowed-agent".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    };
    let result = engine.check_agent_effective_permissions("allowed-agent", &body);
    assert!(result.is_none());
}

#[test]
fn test_check_agent_effective_permissions_slash_command() {
    let engine = make_engine();
    // Even with agent in cache, SlashCommand → None (dimension_name returns None)
    {
        let mut cache = engine.agent_permissions.write().unwrap();
        cache.insert(
            "cached-agent".to_string(),
            AgentPermissions {
                agent_id: "cached-agent".to_string(),
                permissions: HashMap::from([(
                    "exec".to_string(),
                    crate::agent::config::ActionPermission {
                        allowed: false,
                        limits: crate::agent::config::PermissionLimits::default(),
                    },
                )]),
                inherited_from: None,
            },
        );
    }

    let body = PermissionRequestBody::SlashCommand {
        agent: "cached-agent".to_string(),
        command: "/status".to_string(),
    };
    let result = engine.check_agent_effective_permissions("cached-agent", &body);
    assert!(result.is_none());
}

// -------------------------------------------------------------------------
// Concurrent test (E)
// -------------------------------------------------------------------------

#[test]
fn test_concurrent_spawn_and_evaluate() {
    use std::sync::Arc;
    use std::thread;

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
