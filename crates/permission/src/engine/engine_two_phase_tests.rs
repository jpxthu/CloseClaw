//! Two-phase evaluation tests for PermissionEngine.
//!
//! Covers all two-phase (Agent × User) intersection scenarios from issue #662.

use super::engine_eval::PermissionEngine;
use super::engine_types::{
    Caller, Effect, MatchType, PermissionRequest, PermissionRequestBody, PermissionResponse, Rule,
    RuleSet, Subject,
};

/// Helper to build a minimal RuleSet with given defaults and rules.
fn make_ruleset(default_file: Effect, rules: Vec<Rule>) -> PermissionEngine {
    let ruleset = RuleSet {
        rules,
        defaults: super::engine_types::Defaults {
            file_read: default_file,
            file_write: default_file,
            exec: default_file,
            network: default_file,
            inter_agent: default_file,
            config: default_file,
            tool_call: default_file,
            message: Effect::Allow,
        },
        user_defaults: super::engine_types::Defaults::user_defaults(),
        template_includes: vec![],
        rule_version: String::new(),
    };
    PermissionEngine::new_with_default_data_root(ruleset)
}

/// Helper to make a FileOp request.
fn file_request(agent: &str, path: &str, user_id: &str) -> PermissionRequest {
    PermissionRequest::WithCaller {
        caller: Caller {
            user_id: user_id.to_string(),
            agent: agent.to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: agent.to_string(),
            path: path.to_string(),
            op: "read".to_string(),
        },
    }
}
// -------------------------------------------------------------------------
// Two-phase evaluation tests
// -------------------------------------------------------------------------
/// Agent Allow + User Allow → Allowed (non-owner)
#[test]
fn test_two_phase_agent_allow_user_allow() {
    let rules = vec![
        Rule {
            name: "agent-allows-read".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "user-allows-read".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                user_match: MatchType::Exact,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 5,
        },
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    let resp = engine.evaluate(file_request("test-agent", "/data/file.txt", "alice"), None);
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
}

/// Agent dimension deny directly returns Denied (non-owner)
#[test]
fn test_two_phase_agent_deny_user_allow() {
    let rules = vec![
        Rule {
            name: "agent-denies-write".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Deny,
            actions: vec![super::engine_types::Action::File {
                operation: "write".to_string(),
                paths: vec!["/etc/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "user-allows-write".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                user_match: MatchType::Exact,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "write".to_string(),
                paths: vec!["/etc/**".to_string()],
            }],
            template: None,
            priority: 5,
        },
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/etc/passwd".to_string(),
                op: "write".to_string(),
            },
        },
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));
}

/// Agent Allow + User no match → Denied (non-owner)
#[test]
fn test_two_phase_agent_allow_user_no_match() {
    let rules = vec![
        Rule {
            name: "agent-allows-read".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        // No UserAndAgent rule for alice+test-agent
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    // Agent Allow + User None → Denied (intersection model: both must Allow)
    let resp = engine.evaluate(file_request("test-agent", "/data/file.txt", "alice"), None);
    assert!(matches!(resp, PermissionResponse::Denied { .. }));
}

/// Two phases both have no match → Denied (default policy)
#[test]
fn test_two_phase_no_match_default_deny() {
    // No rules at all
    let engine = make_ruleset(Effect::Deny, vec![]);
    let resp = engine.evaluate(file_request("unknown-agent", "/data/file.txt", "bob"), None);
    assert!(matches!(resp, PermissionResponse::Denied { .. }));
}

/// Owner + Agent Allow → Allowed (skip User phase)
#[test]
fn test_two_phase_owner_agent_allow() {
    let rules = vec![
        Rule {
            name: "owner-agent-allows-read".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        // No UserAndAgent rules (owner skips user phase)
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    let resp = engine.evaluate(file_request("test-agent", "/etc/passwd", "owner"), None);
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
}

/// Owner + Agent Deny → Denied
#[test]
fn test_two_phase_owner_agent_deny() {
    let rules = vec![Rule {
        name: "owner-agent-denies".to_string(),
        subject: Subject::AgentOnly {
            agent: "test-agent".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Deny,
        actions: vec![super::engine_types::Action::File {
            operation: "write".to_string(),
            paths: vec!["/etc/**".to_string()],
        }],
        template: None,
        priority: 10,
    }];
    let engine = make_ruleset(Effect::Deny, rules);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "owner".to_string(),
                agent: "test-agent".to_string(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/etc/passwd".to_string(),
                op: "write".to_string(),
            },
        },
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));
}

/// Owner + Agent no match → Denied (default policy)
#[test]
fn test_two_phase_owner_agent_no_match() {
    // No rules at all
    let engine = make_ruleset(Effect::Deny, vec![]);
    // Owner still gets default deny when no rules match
    let resp = engine.evaluate(
        file_request("unknown-agent", "/data/file.txt", "owner"),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));
}

/// Non-owner behavior not affected by Owner shortcut (User Allow alone is not enough)
#[test]
fn test_two_phase_non_owner_needs_both_phases() {
    // Only UserAndAgent Allow rule, no AgentOnly rule
    let rules = vec![Rule {
        name: "user-allows-read".to_string(),
        subject: Subject::UserAndAgent {
            user_id: "alice".to_string(),
            agent: "test-agent".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![super::engine_types::Action::File {
            operation: "read".to_string(),
            paths: vec!["/data/**".to_string()],
        }],
        template: None,
        priority: 5,
    }];
    let engine = make_ruleset(Effect::Deny, rules);
    // Alice is not owner, Agent phase has no match, User phase Allow is not enough alone
    let resp = engine.evaluate(file_request("test-agent", "/data/file.txt", "alice"), None);
    // Agent None + User Allow → Denied (intersection model: both must Allow)
    assert!(matches!(resp, PermissionResponse::Denied { .. }));
}
// ---------------------------------------------------------------------------
// Extra deny subjects tests
// ---------------------------------------------------------------------------
/// extra_deny_subjects = None → Agent Allow + User no match → Denied (intersection model)
#[test]
fn test_extra_deny_subjects_empty() {
    let rules = vec![Rule {
        name: "agent-allows-read".to_string(),
        subject: Subject::AgentOnly {
            agent: "test-agent".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![super::engine_types::Action::File {
            operation: "read".to_string(),
            paths: vec!["/data/**".to_string()],
        }],
        template: None,
        priority: 10,
    }];
    let engine = make_ruleset(Effect::Deny, rules);
    // Only AgentOnly Allow rule, no UserAndAgent rule → intersection model → Denied
    let resp = engine.evaluate(file_request("test-agent", "/data/file.txt", "alice"), None);
    assert!(matches!(resp, PermissionResponse::Denied { .. }));
}

/// extra_deny_subjects has a matching subject → overrides result to Denied
#[test]
fn test_extra_deny_subjects_match() {
    let rules = vec![Rule {
        name: "agent-allows-read".to_string(),
        subject: Subject::AgentOnly {
            agent: "test-agent".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![super::engine_types::Action::File {
            operation: "read".to_string(),
            paths: vec!["/data/**".to_string()],
        }],
        template: None,
        priority: 10,
    }];
    let engine = make_ruleset(Effect::Deny, rules);
    let extra = vec![Subject::AgentOnly {
        agent: "test-agent".to_string(),
        match_type: MatchType::Exact,
    }];
    let resp = engine.evaluate(
        file_request("test-agent", "/data/file.txt", "alice"),
        Some(extra),
    );
    match resp {
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("parent agent restriction"));
            assert_eq!(rule, "<extra_deny>");
        }
        _ => panic!("expected Denied, got {:?}", resp),
    };
}

/// extra_deny_subjects has a subject but does NOT match caller → normal Allow
#[test]
fn test_extra_deny_subjects_no_match() {
    let rules = vec![
        Rule {
            name: "agent-allows-read".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "user-allows-read".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                user_match: MatchType::Exact,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    let extra = vec![Subject::AgentOnly {
        agent: "other-agent".to_string(),
        match_type: MatchType::Exact,
    }];
    let resp = engine.evaluate(
        file_request("test-agent", "/data/file.txt", "alice"),
        Some(extra),
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
}

/// Normal two-phase result is Allow, but extra_deny matches → overrides to Denied
#[test]
fn test_extra_deny_overrides_allow() {
    let rules = vec![
        Rule {
            name: "agent-allows-read".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "user-allows-read".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                user_match: MatchType::Exact,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    let extra = vec![Subject::AgentOnly {
        agent: "test-agent".to_string(),
        match_type: MatchType::Exact,
    }];
    let resp = engine.evaluate(
        file_request("test-agent", "/data/file.txt", "alice"),
        Some(extra),
    );
    assert!(matches!(resp, PermissionResponse::Denied { reason, .. }
            if reason.contains("parent agent restriction")));
}
// ---------------------------------------------------------------------------
// get_agent_deny_subjects tests
// ---------------------------------------------------------------------------
/// get_agent_deny_subjects extracts parent AgentOnly Deny rules, replacing agent with child_id
#[test]
fn test_get_agent_deny_subjects_basic() {
    let rules = vec![
        Rule {
            name: "parent-deny-spawn".to_string(),
            subject: Subject::AgentOnly {
                agent: "parent-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Deny,
            actions: vec![super::engine_types::Action::ToolCall {
                skill: "*".to_string(),
                methods: vec![],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "parent-allow-read".to_string(),
            subject: Subject::AgentOnly {
                agent: "parent-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/**".to_string()],
            }],
            template: None,
            priority: 5,
        },
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    let subjects = engine.get_agent_deny_subjects("parent-agent", "child-agent");
    assert_eq!(subjects.len(), 1);
    let replaced = &subjects[0];
    assert!(matches!(replaced, Subject::AgentOnly { agent, .. } if agent == "child-agent"));
}

/// Parent agent has no deny rules → returns empty
#[test]
fn test_get_agent_deny_subjects_empty() {
    let rules = vec![Rule {
        name: "parent-allow-read".to_string(),
        subject: Subject::AgentOnly {
            agent: "parent-agent".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![super::engine_types::Action::File {
            operation: "read".to_string(),
            paths: vec!["/**".to_string()],
        }],
        template: None,
        priority: 5,
    }];
    let engine = make_ruleset(Effect::Deny, rules);
    let subjects = engine.get_agent_deny_subjects("parent-agent", "child-agent");
    assert!(subjects.is_empty());
}

// ---------------------------------------------------------------------------
// MessageSend two-phase default tests
// ---------------------------------------------------------------------------

/// MessageSend with no rules → defaults to Allow (design doc contract)
#[test]
fn test_message_send_no_rules_defaults_to_allow() {
    // Owner caller: agent-only evaluation, message default Allow
    let engine = make_ruleset(Effect::Deny, vec![]);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "owner".to_string(),
                agent: "test-agent".to_string(),
            },
            request: PermissionRequestBody::MessageSend {
                agent: "test-agent".to_string(),
                direction: super::engine_types::MessageDirection::Send,
                target: "chat_1".to_string(),
            },
        },
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "MessageSend with no rules should default to Allow, got {:?}",
        resp
    );
}

/// Non-owner MessageSend with no rules → user_defaults.message = Deny,
/// so user phase fallback is Deny → intersection is Denied.
#[test]
fn test_message_send_non_owner_no_rules() {
    let engine = make_ruleset(Effect::Deny, vec![]);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
            },
            request: PermissionRequestBody::MessageSend {
                agent: "test-agent".to_string(),
                direction: super::engine_types::MessageDirection::Send,
                target: "chat_1".to_string(),
            },
        },
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Non-owner MessageSend should default to Denied (user_defaults), got {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// user_defaults tests
// ---------------------------------------------------------------------------

/// Defaults::user_defaults() returns all Deny (including message).
#[test]
fn test_user_defaults_returns_all_deny() {
    let ud = super::engine_types::Defaults::user_defaults();
    assert_eq!(ud.file_read, Effect::Deny);
    assert_eq!(ud.file_write, Effect::Deny);
    assert_eq!(ud.exec, Effect::Deny);
    assert_eq!(ud.network, Effect::Deny);
    assert_eq!(ud.inter_agent, Effect::Deny);
    assert_eq!(ud.config, Effect::Deny);
    assert_eq!(ud.tool_call, Effect::Deny);
    assert_eq!(ud.message, Effect::Deny);
}

/// Non-owner with no rules: file read defaults to Denied via user_defaults.
#[test]
fn test_non_owner_no_rules_file_command_deny() {
    let engine = make_ruleset(Effect::Allow, vec![]);
    let file_resp = engine.evaluate(file_request("test-agent", "/data/file.txt", "alice"), None);
    assert!(
        matches!(file_resp, PermissionResponse::Denied { .. }),
        "Non-owner file read should be Denied via user_defaults, got {:?}",
        file_resp
    );
    let cmd_resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
            },
            request: PermissionRequestBody::CommandExec {
                agent: "test-agent".to_string(),
                cmd: "ls".to_string(),
                args: vec![],
            },
        },
        None,
    );
    assert!(
        matches!(cmd_resp, PermissionResponse::Denied { .. }),
        "Non-owner command exec should be Denied via user_defaults, got {:?}",
        cmd_resp
    );
}

// ---------------------------------------------------------------------------
// Step 1.3: UserOnly intersection boundary tests
// ---------------------------------------------------------------------------

/// UserOnly Allow + no Agent rules → Denied (default all Deny).
/// Boundary: User phase Allow without Agent phase Allow is not sufficient.
#[test]
fn test_user_only_allow_no_agent_rules_denied() {
    let rules = vec![Rule {
        name: "user-only-read-allow".to_string(),
        subject: Subject::UserOnly {
            user_id: "alice".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![super::engine_types::Action::File {
            operation: "read".to_string(),
            paths: vec!["/data/**".to_string()],
        }],
        template: None,
        priority: 10,
    }];
    let engine = make_ruleset(Effect::Deny, rules);
    // User phase: UserOnly Allow → Some(Allowed)
    // Agent phase: no match → None
    // Intersection: (None, Some(Allowed)) → falls to defaults → user_defaults → Denied
    let resp = engine.evaluate(file_request("any-agent", "/data/file.txt", "alice"), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "UserOnly Allow without Agent Allow should be Denied, got: {:?}",
        resp
    );
}

/// UserOnly Deny + AgentOnly Allow → Denied (any Deny → Deny).
/// Cross verification: User phase Deny overrides Agent phase Allow.
#[test]
fn test_user_only_deny_agent_allow_denied() {
    let rules = vec![
        Rule {
            name: "agent-read-allow".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "user-read-deny".to_string(),
            subject: Subject::UserOnly {
                user_id: "alice".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Deny,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 5,
        },
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    let resp = engine.evaluate(file_request("test-agent", "/data/file.txt", "alice"), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "UserOnly Deny should override Agent Allow → Denied, got: {:?}",
        resp
    );
}

/// Owner (user_id = "owner") + UserOnly rule → exempt User dimension.
/// Owner shortcut skips User phase; only Agent dimension matters.
#[test]
fn test_owner_user_only_rule_exempt_user_dimension() {
    let rules = vec![
        Rule {
            name: "agent-read-allow".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 10,
        },
        Rule {
            name: "user-read-deny".to_string(),
            subject: Subject::UserOnly {
                user_id: "owner".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Deny,
            actions: vec![super::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["/data/**".to_string()],
            }],
            template: None,
            priority: 5,
        },
    ];
    let engine = make_ruleset(Effect::Deny, rules);
    // Owner: Agent Allow → Allowed (User phase skipped)
    let resp = engine.evaluate(file_request("test-agent", "/data/file.txt", "owner"), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Owner with Agent Allow should be Allowed (User phase skipped), got: {:?}",
        resp
    );
}

/// Empty user_id + Agent Allow → Allowed (no user phase evaluation).
/// Regression: empty user_id means User phase is skipped, Agent result wins.
#[test]
fn test_empty_user_id_agent_allow_allowed() {
    let rules = vec![Rule {
        name: "agent-read-allow".to_string(),
        subject: Subject::AgentOnly {
            agent: "test-agent".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![super::engine_types::Action::File {
            operation: "read".to_string(),
            paths: vec!["/data/**".to_string()],
        }],
        template: None,
        priority: 10,
    }];
    let engine = make_ruleset(Effect::Deny, rules);
    let resp = engine.evaluate(file_request("test-agent", "/data/file.txt", ""), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Empty user_id + Agent Allow should be Allowed, got: {:?}",
        resp
    );
}

/// RuleSet deserialization without user_defaults field → auto-fills all Deny.
#[test]
fn test_ruleset_deserialize_without_user_defaults() {
    let json = r#"{
        "rules": [],
        "defaults": {"message": "allow"}
    }"#;
    let ruleset: super::engine_types::RuleSet = serde_json::from_str(json).unwrap();
    assert_eq!(ruleset.user_defaults.file_read, Effect::Deny);
    assert_eq!(ruleset.user_defaults.file_write, Effect::Deny);
    assert_eq!(ruleset.user_defaults.exec, Effect::Deny);
    assert_eq!(ruleset.user_defaults.network, Effect::Deny);
    assert_eq!(ruleset.user_defaults.inter_agent, Effect::Deny);
    assert_eq!(ruleset.user_defaults.config, Effect::Deny);
    assert_eq!(ruleset.user_defaults.tool_call, Effect::Deny);
    assert_eq!(ruleset.user_defaults.message, Effect::Deny);
}
