use super::engine_eval::PermissionEngine;
use super::engine_types::{
    Caller, Effect, MatchType, PermissionRequest, PermissionRequestBody, PermissionResponse,
    Subject,
};
use crate::actions::ActionBuilder;
use crate::rules::{RuleBuilder, RuleSetBuilder};

// -------------------------------------------------------------------------
// Owner shortcut tests
// -------------------------------------------------------------------------

fn owner_evaluate(request_body: PermissionRequestBody) -> PermissionResponse {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("agent-only-allow-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("agent-only-deny-etc")
                .subject_agent("test-agent")
                .deny()
                .action(
                    ActionBuilder::file("write", vec!["/etc/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-agent-deny-data-write")
                .subject_user_and_agent("owner", "test-agent", MatchType::Exact, MatchType::Exact)
                .deny()
                .action(
                    ActionBuilder::file("write", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "owner".to_string(),
            agent: "test-agent".to_string(),
        },
        request: request_body,
        owner_id: Some("owner".to_string()),
    };
    engine.evaluate(request, None)
}

#[test]
fn test_owner_agent_only_allow() {
    // owner + AgentOnly allow rule → Allowed
    let resp = owner_evaluate(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/data/some-file".to_string(),
        op: "read".to_string(),
    });
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "owner should be allowed by AgentOnly allow rule: {:?}",
        resp
    );
}

#[test]
fn test_owner_agent_only_deny() {
    // owner + AgentOnly deny rule → Denied
    let resp = owner_evaluate(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/etc/shadow".to_string(),
        op: "write".to_string(),
    });
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "owner should be denied by AgentOnly deny rule: {:?}",
        resp
    );
}

#[test]
fn test_owner_not_affected_by_user_and_agent_deny() {
    // owner + UserAndAgent deny → Allowed (UserAndAgent rules skipped)
    // The UserAndAgent deny rule on file_write /data/** should NOT affect owner.
    // Since there's no matching AgentOnly rule for this action, it should fall
    // through to default deny.
    // But wait - there IS an AgentOnly allow rule for file_read /data/**.
    // For file_write /data/**, no AgentOnly rule matches, so default deny.
    let resp = owner_evaluate(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/data/other-file".to_string(),
        op: "write".to_string(),
    });
    // The UserAndAgent deny should NOT trigger (it's skipped for owner).
    // No AgentOnly rule matches this write, so falls through to default Deny.
    // The denials are from default policy, NOT from the UserAndAgent deny rule.
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "default"),
        "owner should NOT be affected by UserAndAgent deny rule: {:?}",
        resp
    );
}

#[test]
fn test_owner_user_and_agent_deny_plus_agent_only_deny() {
    // owner + UserAndAgent deny + AgentOnly deny → Denied (AgentOnly deny still works)
    let resp = owner_evaluate(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/etc/config".to_string(),
        op: "write".to_string(),
    });
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "owner should be denied by AgentOnly deny rule even with UserAndAgent deny present: {:?}",
        resp
    );
}

#[test]
fn test_owner_no_matching_rule_default_deny() {
    // owner with no matching AgentOnly rules → default deny
    let resp = owner_evaluate(PermissionRequestBody::NetOp {
        agent: "test-agent".to_string(),
        host: "example.com".to_string(),
        port: 443,
    });
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "default"),
        "owner with no matching rule should get default deny: {:?}",
        resp
    );
}

#[test]
fn test_non_owner_unaffected_by_owner_shortcut() {
    // Non-owner caller should still evaluate UserAndAgent rules normally
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_exec(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("non-owner-user-agent-deny")
                .subject_user_and_agent("alice", "test-agent", MatchType::Exact, MatchType::Exact)
                .deny()
                .action(
                    ActionBuilder::file("read", vec!["/secret/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // alice should be denied by UserAndAgent deny rule
    let alice_req = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "alice".to_string(),
            agent: "test-agent".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/secret/data".to_string(),
            op: "read".to_string(),
        },
        owner_id: None,
    };
    let alice_resp = engine.evaluate(alice_req, None);
    assert!(
        matches!(alice_resp, PermissionResponse::Denied { .. }),
        "non-owner should be denied by UserAndAgent deny rule: {:?}",
        alice_resp
    );

    // bob (different user_id) is not affected by alice's deny rule,
    // but gets user_defaults (all-Deny) since no rule matches bob.
    let bob_req = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "bob".to_string(),
            agent: "test-agent".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/secret/data".to_string(),
            op: "read".to_string(),
        },
        owner_id: None,
    };
    let bob_resp = engine.evaluate(bob_req, None);
    assert!(
        matches!(bob_resp, PermissionResponse::Denied { ref rule, .. } if rule == "default"),
        "bob (different user) should get user_defaults deny (all-Deny), not alice's rule: {:?}",
        bob_resp
    );
}

// -------------------------------------------------------------------------
// Data-driven Owner judgment (injected session-bound owner_id)
// -------------------------------------------------------------------------

/// A UserOnly deny rule targeting the caller applies when no session
/// Owner id is injected — the caller's user_id alone never triggers the
/// Owner shortcut anymore (no literal `"owner"` judgment).
#[test]
fn test_no_injected_owner_id_shortcut_not_applied() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("owner-user-only-deny")
                .subject_user_and_agent("owner", "test-agent", MatchType::Exact, MatchType::Exact)
                .deny()
                .action(
                    ActionBuilder::file("read", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "owner".to_string(),
            agent: "test-agent".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/data/file.txt".to_string(),
            op: "read".to_string(),
        },
        owner_id: None,
    };
    let resp = engine.evaluate(request, None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "no injected owner_id → no Owner shortcut, User-phase deny applies: {:?}",
        resp
    );
}

/// A caller whose user_id equals the injected session-bound Owner id
/// qualifies for the Owner shortcut even when the id is not the literal
/// `"owner"` (e.g. a Feishu open_id or terminal uid).
#[test]
fn test_injected_owner_id_shortcut_applies_for_non_literal_owner() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("agent-only-allow-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-only-deny-read")
                .subject(Subject::UserOnly {
                    user_id: "ou_real_owner".to_string(),
                    match_type: MatchType::Exact,
                })
                .deny()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_real_owner".to_string(),
            agent: "test-agent".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/data/file.txt".to_string(),
            op: "read".to_string(),
        },
        owner_id: Some("ou_real_owner".to_string()),
    };
    let resp = engine.evaluate(request, None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "caller == injected session Owner → shortcut skips User phase: {:?}",
        resp
    );
}

/// A caller whose user_id differs from the injected session-bound Owner
/// id does not qualify for the Owner shortcut — the User phase runs and
/// vetoes the Agent Allow. If the shortcut leaked to non-owner senders
/// this request would come back Allowed.
#[test]
fn test_non_owner_caller_with_injected_owner_id_no_shortcut() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("agent-only-allow-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-phase-deny-read")
                .subject_user_and_agent("alice", "test-agent", MatchType::Exact, MatchType::Exact)
                .deny()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "alice".to_string(),
            agent: "test-agent".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/data/file.txt".to_string(),
            op: "read".to_string(),
        },
        owner_id: Some("ou_real_owner".to_string()),
    };
    let resp = engine.evaluate(request, None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "sender ≠ injected session Owner → no shortcut, User phase applies: {:?}",
        resp
    );
}

/// The literal `"owner"` caller id grants nothing by itself: with the
/// session Owner bound to a non-literal id, a caller literally named
/// `"owner"` is judged by the User phase like any other sender. Under
/// the former literal judgment this request would come back Allowed.
#[test]
fn test_literal_owner_caller_not_shortcut_when_owner_id_differs() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("agent-only-allow-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-only-deny-literal-owner")
                .subject(Subject::UserOnly {
                    user_id: "owner".to_string(),
                    match_type: MatchType::Exact,
                })
                .deny()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "owner".to_string(),
            agent: "test-agent".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/data/file.txt".to_string(),
            op: "read".to_string(),
        },
        owner_id: Some("ou_real_owner".to_string()),
    };
    let resp = engine.evaluate(request, None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "literal \"owner\" caller with a different session Owner must be \
         judged by the User phase, not short-circuited: {:?}",
        resp
    );
}
