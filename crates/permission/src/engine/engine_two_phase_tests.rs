//! Two-phase evaluation tests for PermissionEngine.
//!
//! Covers all two-phase (Agent × User) intersection scenarios from issue #662.

use super::engine_eval::PermissionEngine;
use super::engine_types::{
    Caller, Effect, MatchType, PermissionRequest, PermissionRequestBody, PermissionResponse, Rule,
    RuleSet, Subject,
};
use std::collections::HashMap;

/// Helper to build a minimal RuleSet with given defaults and rules.
fn make_ruleset(default_file: Effect, rules: Vec<Rule>) -> PermissionEngine {
    let ruleset = RuleSet {
        rules,
        defaults: super::engine_types::Defaults {
            file: default_file,
            command: default_file,
            network: default_file,
            inter_agent: default_file,
            config: default_file,
            tool_call: default_file,
            message: Effect::Allow,
        },
        template_includes: vec![],
        agent_creators: HashMap::new(),
    };
    PermissionEngine::new_with_default_data_root(ruleset)
}

/// Helper to make a FileOp request.
fn file_request(agent: &str, path: &str, user_id: &str) -> PermissionRequest {
    PermissionRequest::WithCaller {
        caller: Caller {
            user_id: user_id.to_string(),
            agent: agent.to_string(),
            creator_id: String::new(),
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
                creator_id: String::new(),
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
                creator_id: String::new(),
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
        PermissionResponse::Denied {
            reason,
            rule,
            risk_level: _,
        } => {
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
                creator_id: String::new(),
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

/// Non-owner MessageSend with no rules → message default Allow, user phase
/// also returns default Allow → intersection is Allow.
#[test]
fn test_message_send_non_owner_no_rules() {
    let engine = make_ruleset(Effect::Deny, vec![]);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                creator_id: String::new(),
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
        "Non-owner MessageSend should default to Allow, got {:?}",
        resp
    );
}
