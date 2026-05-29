use super::engine_eval::PermissionEngine;
use super::engine_types::{
    Caller, Effect, MatchType, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use crate::permission::actions::ActionBuilder;
use crate::permission::rules::{RuleBuilder, RuleSetBuilder};

// -------------------------------------------------------------------------
// Owner shortcut tests
// -------------------------------------------------------------------------

fn owner_evaluate(request_body: PermissionRequestBody) -> PermissionResponse {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
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
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "owner".to_string(),
            agent: "test-agent".to_string(),
            creator_id: String::new(),
        },
        request: request_body,
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
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
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
    let engine = PermissionEngine::new(ruleset);

    // alice should be denied by UserAndAgent deny rule
    let alice_req = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "alice".to_string(),
            agent: "test-agent".to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/secret/data".to_string(),
            op: "read".to_string(),
        },
    };
    let alice_resp = engine.evaluate(alice_req, None);
    assert!(
        matches!(alice_resp, PermissionResponse::Denied { .. }),
        "non-owner should be denied by UserAndAgent deny rule: {:?}",
        alice_resp
    );

    // bob should NOT be affected (different user_id)
    let bob_req = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "bob".to_string(),
            agent: "test-agent".to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/secret/data".to_string(),
            op: "read".to_string(),
        },
    };
    let bob_resp = engine.evaluate(bob_req, None);
    assert!(
        matches!(bob_resp, PermissionResponse::Allowed { .. }),
        "bob (different user) should get default allow, not blocked by alice's rule: {:?}",
        bob_resp
    );
}

#[test]
fn test_owner_shortcut_after_creator_rule() {
    // Creator rule should take priority over owner shortcut.
    // But if caller is both owner AND creator, they get Allowed via creator rule.
    let ruleset = RuleSetBuilder::new()
        .agent_creator("test-agent", "owner")
        .default_file(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "owner".to_string(),
            agent: "test-agent".to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec![],
        },
    };
    let resp = engine.evaluate(request, None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "owner who is also creator gets Allowed via creator rule (higher priority): {:?}",
        resp
    );
}
