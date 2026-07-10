use std::collections::HashMap;

use super::engine_eval::PermissionEngine;
use super::engine_test_providers::HashMapProvider;
use super::engine_types::{Effect, PermissionRequest, PermissionRequestBody, PermissionResponse};
use crate::actions::ActionBuilder;
use crate::mock_session_lookup::MockSessionLookup;
use crate::rules::{RuleBuilder, RuleSetBuilder};
use closeclaw_config::agents::{ActionPermission, AgentPermissions, PermissionLimits};

// -------------------------------------------------------------------------
// Test helpers
// -------------------------------------------------------------------------

fn make_engine_with_defaults() -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_config(Effect::Allow)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset)
}

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

fn make_all_allowed(agent_id: &str) -> AgentPermissions {
    make_perms(
        agent_id,
        &[
            ("command", true),
            ("file_read", true),
            ("file_write", true),
            ("network", true),
            ("spawn", true),
            ("tool_call", true),
            ("config_write", true),
        ],
    )
}

async fn make_session_lookup() -> MockSessionLookup {
    MockSessionLookup::new()
}

// register_session and register_parent_child replaced by MockSessionLookup methods

// -------------------------------------------------------------------------
// Test 1: Chain intersection narrows dimension (parent Deny → child blocked)
// -------------------------------------------------------------------------

/// Parent denies file_write in its configured permissions.
/// Child tries file_write → denied by chain intersection.
#[tokio::test]
async fn test_chain_intersection_file_write_denied_by_parent() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    // Parent denies file_write, allows everything else
    perms.insert(
        "parent".to_string(),
        make_perms(
            "parent",
            &[
                ("command", true),
                ("file_read", true),
                ("file_write", false),
                ("network", true),
                ("spawn", true),
                ("tool_call", true),
                ("config_write", true),
            ],
        ),
    );
    // Child allows everything
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "child".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { ref reason, .. } if reason.contains("chain")),
        "file_write should be denied by parent chain intersection: {:?}",
        resp
    );
}

/// Parent denies file_write, child tries file_read → allowed (different dimension).
#[tokio::test]
async fn test_chain_intersection_file_read_allowed_when_only_write_denied() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms(
            "parent",
            &[
                ("command", true),
                ("file_read", true),
                ("file_write", false),
                ("network", true),
                ("spawn", true),
                ("tool_call", true),
                ("config_write", true),
            ],
        ),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "child".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "read".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "file_read should be allowed (parent only denies file_write): {:?}",
        resp
    );
}

/// Single parent denies file_write → child blocked.
/// Verifies single-level chain works, not just multi-level.
#[tokio::test]
async fn test_single_parent_denies_file_write() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms(
            "parent",
            &[
                ("command", true),
                ("file_read", true),
                ("file_write", false),
            ],
        ),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "child".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "file_write should be denied by single parent: {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 2: No chain → behavior equivalent to evaluate()
// -------------------------------------------------------------------------

/// Root agent (no parent) → evaluate_with_chain behaves like evaluate.
/// The chain intersection check is skipped, and the response matches evaluate().
#[tokio::test]
async fn test_no_chain_matches_evaluate() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("root-session", "root");
    // No parent registered → no chain

    let mut perms = HashMap::new();
    perms.insert("root".to_string(), make_all_allowed("root"));

    let engine = make_engine_with_defaults();

    let req_chain = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "root".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    });
    let resp_chain = engine
        .evaluate_with_chain(
            req_chain,
            &lookup,
            "root-session",
            &HashMapProvider::new(perms),
        )
        .await;

    // Same request evaluated directly
    let req_direct = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "root".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    });
    let resp_direct = engine.evaluate(req_direct, None);

    assert_eq!(
        std::mem::discriminant(&resp_chain),
        std::mem::discriminant(&resp_direct),
        "no-chain evaluate_with_chain should match evaluate: chain={:?}, direct={:?}",
        resp_chain,
        resp_direct
    );
}

// -------------------------------------------------------------------------
// Test 3: Deny subject propagation still works
// -------------------------------------------------------------------------

/// Parent AgentOnly deny rule propagated to child via chain deny subjects.
/// Parent has a deny rule for "parent" on tool_call → rewritten to "child"
/// → child tool_call denied via extra_deny subjects.
#[tokio::test]
async fn test_deny_subject_propagation_through_chain() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_config(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("parent-deny-toolcall")
                .subject_agent("parent")
                .deny()
                .action(ActionBuilder::tool_call("*").build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert("parent".to_string(), make_all_allowed("parent"));
    perms.insert("child".to_string(), make_all_allowed("child"));

    // Child tries tool_call — should be denied by chain deny propagation
    let req = PermissionRequest::Bare(PermissionRequestBody::ToolCall {
        agent: "child".to_string(),
        skill: "some_tool".to_string(),
        method: "run".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "child tool_call should be denied by chain deny propagation: {:?}",
        resp
    );
}

/// Parent deny propagation: child exec is also denied because the deny
/// subject `AgentOnly { agent: "child" }` matches all actions for "child",
/// not just tool_call. This is the expected behavior of deny subject propagation.
#[tokio::test]
async fn test_deny_propagation_blocks_all_actions_for_child() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_config(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("parent-deny-toolcall")
                .subject_agent("parent")
                .deny()
                .action(ActionBuilder::tool_call("*").build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert("parent".to_string(), make_all_allowed("parent"));
    perms.insert("child".to_string(), make_all_allowed("child"));

    // Child exec is also denied because the deny subject
    // `AgentOnly { agent: "child" }` matches all actions for "child".
    let req_exec = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
        agent: "child".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    });
    let resp_exec = engine
        .evaluate_with_chain(
            req_exec,
            &lookup,
            "child-session",
            &HashMapProvider::new(perms.clone()),
        )
        .await;
    assert!(
        matches!(resp_exec, PermissionResponse::Denied { .. }),
        "child exec should also be denied by chain deny propagation: {:?}",
        resp_exec
    );

    // Child file_read is also denied (same deny subject applies to all actions)
    let req_read = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "child".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "read".to_string(),
    });
    let resp_read = engine
        .evaluate_with_chain(
            req_read,
            &lookup,
            "child-session",
            &HashMapProvider::new(perms),
        )
        .await;
    assert!(
        matches!(resp_read, PermissionResponse::Denied { .. }),
        "child file_read should also be denied by chain deny propagation: {:?}",
        resp_read
    );
}

// -------------------------------------------------------------------------
// Test 4: Owner skip — owner path not blocked by chain intersection
// -------------------------------------------------------------------------

/// Owner session with no parent chain → no chain intersection applied.
/// Owner gets the same result as direct evaluate().
#[tokio::test]
async fn test_owner_no_chain_not_blocked() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("owner-session", "owner");
    // No parent → no chain intersection

    let mut perms = HashMap::new();
    perms.insert("owner".to_string(), make_all_allowed("owner"));

    let engine = make_engine_with_defaults();

    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "owner".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "owner-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "owner with no chain should not be blocked: {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 5: Dimension coverage — exec
// -------------------------------------------------------------------------

/// Parent denies exec → child exec blocked by chain intersection.
#[tokio::test]
async fn test_chain_intersection_exec_denied() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms("parent", &[("command", false)]),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
        agent: "child".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "exec should be denied by parent chain: {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 5: Dimension coverage — network
// -------------------------------------------------------------------------

/// Parent denies network → child network blocked by chain intersection.
#[tokio::test]
async fn test_chain_intersection_network_denied() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms("parent", &[("network", false)]),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::NetOp {
        agent: "child".to_string(),
        host: "example.com".to_string(),
        port: 443,
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "network should be denied by parent chain: {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 5: Dimension coverage — spawn
// -------------------------------------------------------------------------

/// Parent denies spawn → child inter-agent msg blocked by chain intersection.
#[tokio::test]
async fn test_chain_intersection_spawn_denied() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms("parent", &[("spawn", false)]),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::InterAgentMsg {
        from: "child".to_string(),
        to: "other-agent".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "spawn should be denied by parent chain: {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 5: Dimension coverage — tool_call
// -------------------------------------------------------------------------

/// Parent denies tool_call → child tool_call blocked by chain intersection.
#[tokio::test]
async fn test_chain_intersection_tool_call_denied() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms("parent", &[("tool_call", false)]),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::ToolCall {
        agent: "child".to_string(),
        skill: "some_skill".to_string(),
        method: "run".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "tool_call should be denied by parent chain: {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 5: Dimension coverage — config_write
// -------------------------------------------------------------------------

/// Parent denies config_write → child config_write blocked by chain intersection.
#[tokio::test]
async fn test_chain_intersection_config_write_denied() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms("parent", &[("config_write", false)]),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
        agent: "child".to_string(),
        config_file: "/etc/config.toml".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "config_write should be denied by parent chain: {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 5: Dimension coverage — SlashCommand (no dimension → not blocked)
// -------------------------------------------------------------------------

/// SlashCommand has no dimension_name() → chain intersection does not block.
#[tokio::test]
async fn test_chain_intersection_slash_command_not_blocked() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    // Parent denies exec — but SlashCommand has no dimension, so not blocked
    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms("parent", &[("command", false)]),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::SlashCommand {
        agent: "child".to_string(),
        command: "/help".to_string(),
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    // SlashCommand has no dimension → chain check skips → evaluate result
    // With default Allow and no deny rules, this should be Allowed
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "SlashCommand should not be blocked by chain (no dimension): {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test 5: Dimension coverage — unknown op (None dimension)
// -------------------------------------------------------------------------

/// FileOp with unknown op → dimension_name() returns None → not blocked.
#[tokio::test]
async fn test_chain_intersection_unknown_op_not_blocked() {
    let mut lookup = make_session_lookup().await;
    lookup.register_session("parent-session", "parent");
    lookup.register_session("child-session", "child");
    lookup.register_parent_child("parent-session", "child-session");

    // Parent denies file_write — but unknown op has no dimension
    let mut perms = HashMap::new();
    perms.insert(
        "parent".to_string(),
        make_perms("parent", &[("file_write", false)]),
    );
    perms.insert("child".to_string(), make_all_allowed("child"));

    let engine = make_engine_with_defaults();
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "child".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "delete".to_string(), // unknown op → None dimension
    });

    let resp = engine
        .evaluate_with_chain(req, &lookup, "child-session", &HashMapProvider::new(perms))
        .await;
    // Unknown op → dimension_name() = None → chain check skips
    // evaluate() with default Allow → Allowed
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "unknown op should not be blocked by chain (no dimension): {:?}",
        resp
    );
}

// -------------------------------------------------------------------------
// Test: Three-level chain intersection
// -------------------------------------------------------------------------

/// Three-level chain intersection:
/// Root denies network and file_write via configured permissions + deny rules.
/// B tries file_write → denied by chain intersection (Root denies file_write).
/// B tries network → denied by chain intersection (Root denies network).
/// Note: Root's deny rules are rewritten to target B via
/// `collect_chain_deny_subjects`, so exec is also denied by the deny
/// subject propagation mechanism (not by chain intersection).
#[tokio::test]
async fn test_three_level_chain_intersection() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_config(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("root-deny-network")
                .subject_agent("root")
                .deny()
                .action(ActionBuilder::network().build().unwrap())
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("root-deny-filewrite")
                .subject_agent("root")
                .deny()
                .action(
                    ActionBuilder::file("write", vec!["/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let mut lookup = make_session_lookup().await;
    lookup.register_session("session-root", "root");
    lookup.register_session("session-a", "agent-a");
    lookup.register_parent_child("session-root", "session-a");
    lookup.register_session("session-b", "agent-b");
    lookup.register_parent_child("session-a", "session-b");

    // Root denies network and file_write; A allows everything
    let mut perms = HashMap::new();
    perms.insert(
        "root".to_string(),
        make_perms("root", &[("network", false), ("file_write", false)]),
    );
    perms.insert("agent-a".to_string(), make_all_allowed("agent-a"));
    perms.insert("agent-b".to_string(), make_all_allowed("agent-b"));

    // B tries file_write → denied by chain intersection (Root denies file_write)
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "agent-b".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    });
    let resp = engine
        .evaluate_with_chain(
            req,
            &lookup,
            "session-b",
            &HashMapProvider::new(perms.clone()),
        )
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "file_write should be denied by chain intersection: {:?}",
        resp
    );

    // B tries network → denied by chain intersection (Root denies network)
    let req = PermissionRequest::Bare(PermissionRequestBody::NetOp {
        agent: "agent-b".to_string(),
        host: "example.com".to_string(),
        port: 443,
    });
    let resp = engine
        .evaluate_with_chain(req, &lookup, "session-b", &HashMapProvider::new(perms))
        .await;
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "network should be denied by chain intersection: {:?}",
        resp
    );
}
