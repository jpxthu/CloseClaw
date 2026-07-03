use super::engine_eval::PermissionEngine;
use super::engine_helpers::collect_chain_effective_permissions;
use super::engine_spawn::SpawnPermissionError;
use super::engine_types::{
    Effect, MatchType, PermissionRequest, PermissionRequestBody, PermissionResponse, Subject,
};
use crate::actions::ActionBuilder;
use crate::mock_session_lookup::MockSessionLookup;
use crate::rules::RuleBuilder;
use crate::rules::RuleSetBuilder;
use closeclaw_common::agent_config::{ActionPermission, AgentPermissions, PermissionLimits};
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
                closeclaw_common::agent_config::ActionPermission {
                    allowed: true,
                    limits: closeclaw_common::agent_config::PermissionLimits::default(),
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
            closeclaw_common::agent_config::ActionPermission {
                allowed: *dim != "exec",
                limits: closeclaw_common::agent_config::PermissionLimits::default(),
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

// -------------------------------------------------------------------------
// Owner spawn: verify User dimension is skipped
// -------------------------------------------------------------------------

/// Owner spawn skips User dimension — child ∩ parent only, user_perms ignored.
/// When user_perms is None (owner path), user deny cannot block the spawn.
#[test]
fn test_owner_spawn_skips_user_dimension() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");
    // user_perms is None (simulates owner path in spawn.rs)
    let result = engine.validate_and_inject_spawn("child", &child, &parent, None, None, None);
    assert!(result.is_ok());
}

/// Non-owner spawn: user deny blocks the spawn when all dims denied.
#[test]
fn test_non_owner_spawn_user_deny_blocks() {
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
        SpawnPermissionError::FullyDeniedWithUser { user_id, .. } => {
            assert_eq!(user_id, "user-1");
        }
        other => panic!("expected FullyDeniedWithUser, got {:?}", other),
    }
}

/// Owner spawn: user deny is ignored, even if user_perms would block all dims.
#[test]
fn test_owner_spawn_ignores_user_deny() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");
    // Even though user would block everything, owner path passes None for user_perms
    let result =
        engine.validate_and_inject_spawn("child", &child, &parent, None, Some("owner"), None);
    assert!(result.is_ok());
}

/// Owner spawn + extra_deny: extra deny still blocks even for owner.
#[test]
fn test_owner_spawn_extra_deny_still_blocks() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");
    let extra = vec![Subject::AgentOnly {
        agent: "child".to_string(),
        match_type: MatchType::Exact,
    }];
    let result = engine.validate_and_inject_spawn(
        "child",
        &child,
        &parent,
        None,
        Some("owner"),
        Some(&extra),
    );
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
// collect_chain_effective_permissions tests
// -------------------------------------------------------------------------

async fn make_session_lookup() -> MockSessionLookup {
    MockSessionLookup::new()
}

// register_session and register_parent_child replaced by MockSessionLookup methods

fn make_perms(agent_id: &str, dims: &[(&str, bool)]) -> AgentPermissions {
    let permissions = dims
        .iter()
        .map(|&(dim, allowed)| {
            (
                dim.to_string(),
                ActionPermission {
                    allowed,
                    limits: PermissionLimits::default(),
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

/// Three-level chain: Root -> A -> B.
/// Root: {exec: allow, file_write: allow, network: deny}
/// A:    {exec: allow, file_write: deny}
/// B:    {exec: allow}
/// B's effective perms should include network: deny (from Root via A)
/// and file_write: deny (from A).
#[tokio::test]
async fn test_three_level_chain_effective_permissions() {
    let mut lookup = make_session_lookup().await;

    lookup.register_session("session-root", "root");
    lookup.register_session("session-a", "agent-a");
    lookup.register_parent_child("session-root", "session-a");
    lookup.register_session("session-b", "agent-b");
    lookup.register_parent_child("session-a", "session-b");

    let mut perms = HashMap::new();
    perms.insert(
        "root".to_string(),
        make_perms(
            "root",
            &[("exec", true), ("file_write", true), ("network", false)],
        ),
    );
    perms.insert(
        "agent-a".to_string(),
        make_perms("agent-a", &[("exec", true), ("file_write", false)]),
    );
    perms.insert(
        "agent-b".to_string(),
        make_perms("agent-b", &[("exec", true)]),
    );

    let result = collect_chain_effective_permissions(&lookup, &perms, "session-a", "agent-a").await;
    assert!(result.is_some(), "should return Some for parent with perms");
    let effective = result.unwrap();

    // Root intersects with A: exec=T∩T→T, file_write=T∩F→F,
    // network=F∩absent→F
    assert!(
        effective.permissions["exec"].allowed,
        "exec should be allowed"
    );
    assert!(
        !effective.permissions["file_write"].allowed,
        "file_write should be denied"
    );
    assert!(
        !effective.permissions["network"].allowed,
        "network denied from Root"
    );
}

/// Single-level spawn: parent has no extra restrictions.
#[tokio::test]
async fn test_single_level_spawn_no_extra_restrictions() {
    let mut lookup = make_session_lookup().await;

    lookup.register_session("session-parent", "parent");
    lookup.register_session("session-child", "child");
    lookup.register_parent_child("session-parent", "session-child");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms(
            "parent",
            &[
                ("exec", true),
                ("file_read", true),
                ("file_write", true),
                ("network", true),
                ("spawn", true),
                ("tool_call", true),
                ("config_write", true),
            ],
        ),
    );

    let result =
        collect_chain_effective_permissions(&lookup, &perms, "session-parent", "parent").await;
    assert!(result.is_some());
    let effective = result.unwrap();

    // No ancestors above parent -> parent's own perms returned
    for dim in &[
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
    ] {
        assert!(
            effective.permissions[*dim].allowed,
            "{} should be allowed",
            dim
        );
    }
}

/// Chain with a fully denied level: Root -> A (fully denied) -> B.
/// A denies everything, so effective perms are fully denied.
#[tokio::test]
async fn test_chain_with_fully_denied_level() {
    let mut lookup = make_session_lookup().await;

    lookup.register_session("session-root", "root");
    lookup.register_session("session-a", "agent-a");
    lookup.register_parent_child("session-root", "session-a");
    lookup.register_session("session-b", "agent-b");
    lookup.register_parent_child("session-a", "session-b");

    let mut perms = HashMap::new();
    perms.insert(
        "root".to_string(),
        make_perms(
            "root",
            &[("exec", true), ("file_write", true), ("network", true)],
        ),
    );
    // A is fully denied
    perms.insert("agent-a".to_string(), make_fully_denied_perms("agent-a"));

    let result = collect_chain_effective_permissions(&lookup, &perms, "session-a", "agent-a").await;
    assert!(result.is_some());
    let effective = result.unwrap();

    assert!(
        effective.is_fully_denied(),
        "A is fully denied, so effective should be fully denied"
    );
}

// -------------------------------------------------------------------------
// Chain deny: root → A → B propagation
// -------------------------------------------------------------------------

/// Three-level chain deny: root's Deny rule propagates to B via extra_deny_subjects.
/// Root has AgentOnly Deny for tool_call on "root-agent".
/// When evaluating B (child-agent) with extra_deny_subjects containing
/// the rewritten deny subject, B is denied.
#[test]
fn test_chain_deny_root_to_b_propagation() {
    let rules = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("root-deny-toolcall")
                .subject_agent("root-agent")
                .deny()
                .action(ActionBuilder::tool_call("*").build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(rules);

    // Simulate collect_chain_deny_subjects traversing root → A → B:
    // Root's deny rule on "root-agent" gets rewritten to "child-agent" (B).
    let chain_deny_subjects = vec![Subject::AgentOnly {
        agent: "child-agent".to_string(),
        match_type: MatchType::Exact,
    }];

    // B tries to execute tool_call — normally allowed by default
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "child-agent".to_string(),
            skill: "some_tool".to_string(),
            method: "run".to_string(),
        }),
        Some(chain_deny_subjects),
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "<extra_deny>"),
        "B should be denied by chain deny from root: {:?}",
        resp
    );
}

/// Chain deny: root deny propagates to B, but A (intermediate) allows the same action.
/// The chain deny subjects accumulate — root's deny for B takes precedence.
#[test]
fn test_chain_deny_accumulates_across_levels() {
    let rules = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("root-deny-toolcall")
                .subject_agent("root-agent")
                .deny()
                .action(ActionBuilder::tool_call("*").build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(rules);

    // Simulate chain deny subjects from root → A → B:
    // Root's deny for A (rewritten to child-agent) + Root's deny for B (rewritten to child-agent)
    // Both target child-agent, deduplication keeps one.
    let chain_deny_subjects = vec![Subject::AgentOnly {
        agent: "child-agent".to_string(),
        match_type: MatchType::Exact,
    }];

    // B tries tool_call — denied by chain deny from root
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "child-agent".to_string(),
            skill: "any".to_string(),
            method: "any".to_string(),
        }),
        Some(chain_deny_subjects),
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "<extra_deny>"),
        "B should be denied: chain deny accumulates: {:?}",
        resp
    );
}

/// Chain deny with glob match: root deny uses glob pattern "root-*",
/// rewritten to "child-*", matching "child-agent".
#[test]
fn test_chain_deny_glob_match() {
    let rules = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("root-deny-toolcall-glob")
                .subject_agent("root-*")
                .deny()
                .action(ActionBuilder::tool_call("*").build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(rules);

    // Simulate glob deny rewrite: root-* → child-*
    let chain_deny_subjects = vec![Subject::AgentOnly {
        agent: "child-*".to_string(),
        match_type: MatchType::Glob,
    }];

    // child-agent matches child-* glob → denied
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "child-agent".to_string(),
            skill: "web".to_string(),
            method: "search".to_string(),
        }),
        Some(chain_deny_subjects.clone()),
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "<extra_deny>"),
        "child-agent should be denied by glob chain deny: {:?}",
        resp
    );

    // other-agent does NOT match child-* → allowed
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "other-agent".to_string(),
            skill: "web".to_string(),
            method: "search".to_string(),
        }),
        Some(chain_deny_subjects),
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "other-agent should NOT be affected by child-* glob deny: {:?}",
        resp
    );
}

/// Chain deny: extra_deny subjects with multiple deny rules from different ancestors.
/// Root denies tool_call, A denies network — both propagate to B.
#[test]
fn test_chain_deny_multiple_ancestors() {
    let rules = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(rules);

    // Simulate chain deny from two ancestors:
    // Root denies tool_call for B, A denies network for B
    let chain_deny_subjects = vec![
        Subject::AgentOnly {
            agent: "child-agent".to_string(),
            match_type: MatchType::Exact,
        },
        Subject::AgentOnly {
            agent: "child-agent".to_string(),
            match_type: MatchType::Exact,
        }, // duplicate — deduplication keeps one
    ];

    // tool_call denied
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "child-agent".to_string(),
            skill: "any".to_string(),
            method: "any".to_string(),
        }),
        Some(chain_deny_subjects.clone()),
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "<extra_deny>"),
        "tool_call should be denied: {:?}",
        resp
    );

    // network also denied (same subject matches any action)
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::NetOp {
            agent: "child-agent".to_string(),
            host: "example.com".to_string(),
            port: 443,
        }),
        Some(chain_deny_subjects),
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "<extra_deny>"),
        "network should also be denied: {:?}",
        resp
    );
}

/// Chain deny with validate_and_inject_spawn: extra deny blocks spawn of child
/// that has full permissions — simulates root deny propagating to grandchild.
#[test]
fn test_chain_deny_blocks_spawn() {
    let engine = make_engine();
    let child = make_allowed_perms("child");
    let parent = make_allowed_perms("parent");
    // Simulate root's deny rule rewritten to target child
    let extra = vec![Subject::AgentOnly {
        agent: "child".to_string(),
        match_type: MatchType::Exact,
    }];
    let result =
        engine.validate_and_inject_spawn("child", &child, &parent, None, None, Some(&extra));
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

/// Chain deny: extra deny with glob pattern blocks spawn via validate_and_inject_spawn.
#[test]
fn test_chain_deny_glob_blocks_spawn() {
    let engine = make_engine();
    let child = make_allowed_perms("dev-child");
    let parent = make_allowed_perms("parent");
    let extra = vec![Subject::AgentOnly {
        agent: "dev-*".to_string(),
        match_type: MatchType::Glob,
    }];
    let result =
        engine.validate_and_inject_spawn("dev-child", &child, &parent, None, None, Some(&extra));
    assert!(result.is_err());
    match result.unwrap_err() {
        SpawnPermissionError::FullyDenied {
            child_agent_id,
            parent_agent_id,
        } => {
            assert_eq!(child_agent_id, "dev-child");
            assert_eq!(parent_agent_id, "parent");
        }
        other => panic!("expected FullyDenied, got {:?}", other),
    }
}

/// Chain deny: glob deny pattern does NOT block non-matching child spawn.
#[test]
fn test_chain_deny_glob_no_match_allows_spawn() {
    let engine = make_engine();
    let child = make_allowed_perms("prod-child");
    let parent = make_allowed_perms("parent");
    // deny pattern is dev-* — prod-child should not match
    let extra = vec![Subject::AgentOnly {
        agent: "dev-*".to_string(),
        match_type: MatchType::Glob,
    }];
    let result =
        engine.validate_and_inject_spawn("prod-child", &child, &parent, None, None, Some(&extra));
    assert!(result.is_ok());
}
