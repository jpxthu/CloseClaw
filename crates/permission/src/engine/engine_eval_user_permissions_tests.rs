//! Unit tests for `evaluate_user_permissions()`.
//!
//! Verifies that the function returns 8 permission dimensions with correct
//! default values, and that boundary inputs (empty user_id / agent_id) do not panic.

use super::engine_eval::PermissionEngine;
use super::engine_types::{Effect, MessageDirection, PermissionRequestBody};
use crate::rules::RuleSetBuilder;
use std::collections::HashMap;

/// The 8 permission dimensions defined in the design doc.
const ALL_DIMENSIONS: &[&str] = &[
    "exec",
    "file_read",
    "file_write",
    "network",
    "spawn",
    "tool_call",
    "config_write",
    "message",
];

/// Build an engine with no rules — user defaults apply.
fn make_engine() -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .default_message(Effect::Deny)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset)
}

// -------------------------------------------------------------------------
// Basic: 8 dimensions returned
// -------------------------------------------------------------------------

/// evaluate_user_permissions() returns exactly 8 dimensions covering all
/// permission types defined in the design doc.
#[test]
fn test_evaluate_user_permissions_returns_8_dimensions() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "agent-1");

    assert_eq!(
        perms.permissions.len(),
        8,
        "expected 8 dimensions, got {}: {:?}",
        perms.permissions.len(),
        perms.permissions.keys().collect::<Vec<_>>()
    );

    for dim in ALL_DIMENSIONS {
        assert!(
            perms.permissions.contains_key(*dim),
            "missing dimension: {}",
            dim
        );
    }
}

// -------------------------------------------------------------------------
// dimension_name() consistency
// -------------------------------------------------------------------------

/// dimension_name() for each PermissionRequestBody variant returns the same
/// key used in the evaluate_user_permissions() HashMap.
#[test]
fn test_dimension_name_matches_evaluate_dimensions() {
    let agent = "test-agent";

    let cases: Vec<(&str, PermissionRequestBody)> = vec![
        (
            "exec",
            PermissionRequestBody::CommandExec {
                agent: agent.to_string(),
                cmd: String::new(),
                args: Vec::new(),
            },
        ),
        (
            "file_read",
            PermissionRequestBody::FileOp {
                agent: agent.to_string(),
                path: String::new(),
                op: "read".to_string(),
            },
        ),
        (
            "file_write",
            PermissionRequestBody::FileOp {
                agent: agent.to_string(),
                path: String::new(),
                op: "write".to_string(),
            },
        ),
        (
            "network",
            PermissionRequestBody::NetOp {
                agent: agent.to_string(),
                host: String::new(),
                port: 0,
            },
        ),
        (
            "spawn",
            PermissionRequestBody::InterAgentMsg {
                from: agent.to_string(),
                to: String::new(),
            },
        ),
        (
            "tool_call",
            PermissionRequestBody::ToolCall {
                agent: agent.to_string(),
                skill: String::new(),
                method: String::new(),
            },
        ),
        (
            "config_write",
            PermissionRequestBody::ConfigWrite {
                agent: agent.to_string(),
                config_file: String::new(),
            },
        ),
        (
            "message",
            PermissionRequestBody::MessageSend {
                agent: agent.to_string(),
                direction: MessageDirection::Both,
                target: String::new(),
            },
        ),
    ];

    for (expected_dim, body) in &cases {
        let actual = body
            .dimension_name()
            .unwrap_or_else(|| panic!("dimension_name() returned None for {:?}", body));
        assert_eq!(
            actual, *expected_dim,
            "dimension_name() mismatch for {:?}",
            body
        );
    }
}

// -------------------------------------------------------------------------
// Default values: all denied (no rules = user_defaults = all Deny)
// -------------------------------------------------------------------------

/// With an empty RuleSet and no UserAndAgent rules, all 8 dimensions should
/// be denied (allowed = false). This verifies the default policy matches the
/// design doc: user_defaults is all-Deny.
#[test]
fn test_evaluate_user_permissions_defaults_all_denied() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "agent-1");

    for dim in ALL_DIMENSIONS {
        let action = perms
            .permissions
            .get(*dim)
            .unwrap_or_else(|| panic!("missing dimension: {}", dim));
        assert!(
            !action.allowed,
            "dimension '{}' should be denied by default, got allowed={}",
            dim, action.allowed
        );
    }
}

// -------------------------------------------------------------------------
// Network default Deny (design doc: "默认 Deny")
// -------------------------------------------------------------------------

/// Network dimension is specifically documented as "默认 Deny".
#[test]
fn test_network_default_deny() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "agent-1");

    let network = perms.permissions.get("network").unwrap();
    assert!(
        !network.allowed,
        "network should default to Deny per design doc"
    );
}

// -------------------------------------------------------------------------
// Config_write default Deny (design doc: "默认 Deny")
// -------------------------------------------------------------------------

/// Config_write dimension is specifically documented as "默认 Deny".
#[test]
fn test_config_write_default_deny() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "agent-1");

    let config_write = perms.permissions.get("config_write").unwrap();
    assert!(
        !config_write.allowed,
        "config_write should default to Deny per design doc"
    );
}

// -------------------------------------------------------------------------
// Message default Deny for user phase
// -------------------------------------------------------------------------

/// Message dimension in user_defaults is Deny (user has no message
/// privileges unless explicitly granted).
#[test]
fn test_message_default_deny_user_phase() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "agent-1");

    let message = perms.permissions.get("message").unwrap();
    assert!(
        !message.allowed,
        "message should default to Deny in user phase"
    );
}

// -------------------------------------------------------------------------
// Boundary: empty user_id does not panic
// -------------------------------------------------------------------------

/// Empty user_id is a valid boundary input; the function must not panic.
#[test]
fn test_evaluate_user_permissions_empty_user_id() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("", "agent-1");

    assert_eq!(perms.permissions.len(), 8);
    assert_eq!(perms.agent_id, "agent-1");
}

// -------------------------------------------------------------------------
// Boundary: empty agent_id does not panic
// -------------------------------------------------------------------------

/// Empty agent_id is a valid boundary input; the function must not panic.
#[test]
fn test_evaluate_user_permissions_empty_agent_id() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "");

    assert_eq!(perms.permissions.len(), 8);
    assert_eq!(perms.agent_id, "");
}

// -------------------------------------------------------------------------
// Boundary: both empty
// -------------------------------------------------------------------------

/// Both user_id and agent_id empty — should return 8 dimensions without panic.
#[test]
fn test_evaluate_user_permissions_both_empty() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("", "");

    assert_eq!(perms.permissions.len(), 8);
    for dim in ALL_DIMENSIONS {
        assert!(
            perms.permissions.contains_key(*dim),
            "missing dimension: {}",
            dim
        );
    }
}

// -------------------------------------------------------------------------
// Per-dimension default values
// -------------------------------------------------------------------------

/// Verify each dimension's default allowed value matches the design doc:
/// - exec: Deny (command whitelist, default no commands allowed)
/// - file_read: Deny
/// - file_write: Deny
/// - network: Deny (explicitly stated in design doc)
/// - spawn: Deny (inter-agent default Deny)
/// - tool_call: Deny
/// - config_write: Deny (explicitly stated in design doc)
/// - message: Deny (user_defaults)
#[test]
fn test_per_dimension_default_values() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "agent-1");

    let expected_defaults: HashMap<&str, bool> = [
        ("exec", false),
        ("file_read", false),
        ("file_write", false),
        ("network", false),
        ("spawn", false),
        ("tool_call", false),
        ("config_write", false),
        ("message", false),
    ]
    .into();

    for (dim, expected_allowed) in &expected_defaults {
        let action = perms
            .permissions
            .get(*dim)
            .unwrap_or_else(|| panic!("missing dimension: {}", dim));
        assert_eq!(
            action.allowed, *expected_allowed,
            "dimension '{}' default mismatch: expected allowed={}, got allowed={}",
            dim, expected_allowed, action.allowed
        );
    }
}

// -------------------------------------------------------------------------
// agent_id is set correctly
// -------------------------------------------------------------------------

/// The returned AgentPermissions should carry the agent_id passed in.
#[test]
fn test_evaluate_user_permissions_agent_id_preserved() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "my-special-agent");

    assert_eq!(perms.agent_id, "my-special-agent");
}

// -------------------------------------------------------------------------
// inherited_from is None
// -------------------------------------------------------------------------

/// evaluate_user_permissions() produces a standalone AgentPermissions with
/// no inheritance chain.
#[test]
fn test_evaluate_user_permissions_no_inheritance() {
    let engine = make_engine();
    let perms = engine.evaluate_user_permissions("user-1", "agent-1");

    assert!(
        perms.inherited_from.is_none(),
        "inherited_from should be None"
    );
}
