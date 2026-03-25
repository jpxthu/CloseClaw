//!
//! User dimension permission tests
//!
//! Tests for Subject::UserAndAgent, Caller, PermissionRequest WithCaller/Bare,
//! Creator Rule short-circuit, template system, and Rule::validate().

use crate::permission::engine::{
    Action, Caller, Effect, MatchType, PermissionEngine, PermissionRequest, PermissionRequestBody,
    PermissionResponse, Rule, Subject, TemplateRef,
};
use crate::permission::rules::{validation, RuleBuilder, RuleSetBuilder};
use crate::permission::templates::{Template, TemplateSubject};

// =============================================================================
// Subject::UserAndAgent tests
// =============================================================================

#[test]
fn test_user_and_agent_subject_matches_both_exact() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_user_mismatch() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_bob".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_agent_mismatch() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "other-agent".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_glob_matching() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_admin_*".to_string(),
        agent: "dev-*".to_string(),
        user_match: MatchType::Glob,
        agent_match: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_admin_john".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_user_and_agent_subject_mixed_match_types() {
    let subject = Subject::UserAndAgent {
        user_id: "ou_123".to_string(),
        agent: "dev-*".to_string(),
        user_match: MatchType::Exact,
        agent_match: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_123".to_string(),
        agent: "dev-agent-99".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));

    let caller = Caller {
        user_id: "ou_456".to_string(),
        agent: "dev-agent-99".to_string(),
        creator_id: String::new(),
    };
    assert!(!subject.matches(&caller));
}

// =============================================================================
// Agent-only subject still works (backward compat)
// =============================================================================

#[test]
fn test_agent_only_subject_exact() {
    let subject = Subject::AgentOnly {
        agent: "dev-agent-01".to_string(),
        match_type: MatchType::Exact,
    };
    let caller = Caller {
        user_id: "ou_anyone".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

#[test]
fn test_agent_only_subject_glob() {
    let subject = Subject::AgentOnly {
        agent: "dev-*".to_string(),
        match_type: MatchType::Glob,
    };
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "dev-agent-01".to_string(),
        creator_id: String::new(),
    };
    assert!(subject.matches(&caller));
}

// =============================================================================
// PermissionRequest envelope tests
// =============================================================================

#[test]
fn test_bare_request_caller_defaults() {
    let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/tmp".to_string(),
        op: "read".to_string(),
    });
    let caller = request.caller();
    assert_eq!(caller.user_id, "");
    assert_eq!(caller.agent, "test-agent");
    assert_eq!(caller.creator_id, "");
}

#[test]
fn test_with_caller_request() {
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: "".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/**".to_string(),
            op: "read".to_string(),
        },
    };
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_alice");
    assert_eq!(caller.agent, "dev-agent-01");
}

#[test]
fn test_bare_deserialize_old_format() {
    let json = r#"{"type": "file_op", "agent": "test-agent", "path": "/tmp", "op": "read"}"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(request, PermissionRequest::Bare(_)));
    let caller = request.caller();
    assert_eq!(caller.user_id, "");
    assert_eq!(caller.agent, "test-agent");
}

#[test]
fn test_with_caller_deserialize_new_format() {
    let json = r#"{
        "caller": {"user_id": "ou_alice", "agent": "dev-agent-01", "creator_id": ""},
        "type": "file_op",
        "agent": "dev-agent-01",
        "path": "/tmp",
        "op": "read"
    }"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(request, PermissionRequest::WithCaller { .. }));
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_alice");
    assert_eq!(caller.agent, "dev-agent-01");
}

#[test]
fn test_with_caller_deserialize_creator_id() {
    let json = r#"{
        "caller": {"user_id": "ou_john", "agent": "dev-agent-01", "creator_id": "ou_john"},
        "type": "command_exec",
        "agent": "dev-agent-01",
        "cmd": "rm",
        "args": ["-rf", "/"]
    }"#;
    let request: PermissionRequest = serde_json::from_str(json).unwrap();
    let caller = request.caller();
    assert_eq!(caller.user_id, "ou_john");
    assert_eq!(caller.creator_id, "ou_john");
}

#[test]
fn test_with_caller_converts_bare() {
    let bare = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/tmp".to_string(),
        op: "read".to_string(),
    });
    let caller = Caller {
        user_id: "ou_alice".to_string(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let with_caller = bare.with_caller(caller);
    assert!(matches!(with_caller, PermissionRequest::WithCaller { .. }));
    assert_eq!(with_caller.caller().user_id, "ou_alice");
}

#[test]
fn test_permission_request_body_agent_id() {
    assert_eq!(
        PermissionRequestBody::FileOp {
            agent: "a".into(),
            path: "/".into(),
            op: "read".into()
        }
        .agent_id(),
        "a"
    );
    assert_eq!(
        PermissionRequestBody::InterAgentMsg {
            from: "a".into(),
            to: "b".into()
        }
        .agent_id(),
        "a"
    );
}

// =============================================================================
// Creator Rule tests
// =============================================================================

#[tokio::test]
async fn test_creator_rule_short_circuit_caller_creator_id() {
    let ruleset = RuleSetBuilder::new()
        .version("2.0")
        .agent_creator("dev-agent-01", "ou_john")
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_john".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: "ou_john".to_string(),
        },
        request: PermissionRequestBody::CommandExec {
            agent: "dev-agent-01".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/".to_string()],
        },
    };
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_creator_rule_short_circuit_agent_creators_map() {
    let ruleset = RuleSetBuilder::new()
        .version("2.0")
        .agent_creator("dev-agent-01", "ou_john")
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_john".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: "".to_string(),
        },
        request: PermissionRequestBody::CommandExec {
            agent: "dev-agent-01".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/".to_string()],
        },
    };
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_creator_rule_not_matching_non_creator() {
    let ruleset = RuleSetBuilder::new()
        .version("2.0")
        .agent_creator("dev-agent-01", "ou_john")
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_bob".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: "".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/etc/passwd".to_string(),
            op: "read".to_string(),
        },
    };
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_creator_rule_priority_over_explicit_deny() {
    let ruleset = RuleSetBuilder::new()
        .version("2.0")
        .agent_creator("dev-agent-01", "ou_john")
        .rule(
            RuleBuilder::new()
                .name("deny-everything")
                .subject_agent("dev-agent-01")
                .deny()
                .action(Action::All)
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_john".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: "ou_john".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/etc/passwd".to_string(),
            op: "read".to_string(),
        },
    };
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_creator_rule_caller_creator_id_takes_precedence() {
    let ruleset = RuleSetBuilder::new()
        .version("2.0")
        .agent_creator("dev-agent-01", "ou_john")
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    // caller.creator_id takes precedence over agent_creators map.
    // alice claims to be the creator (creator_id = "ou_alice").
    // Since caller.user_id == creator_id, creator rule matches → Allowed.
    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: "ou_alice".to_string(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/etc/passwd".to_string(),
            op: "read".to_string(),
        },
    };
    let response = engine.evaluate(request);
    // Creator rule matches: caller.user_id == caller.creator_id → Allowed
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

// =============================================================================
// User+Agent rule evaluation tests
// =============================================================================

#[tokio::test]
async fn test_user_and_agent_rule_matching() {
    let ruleset = RuleSetBuilder::new()
        .version("2.0")
        .rule(
            RuleBuilder::new()
                .name("alice-read")
                .subject_user_and_agent(
                    "ou_alice",
                    "dev-agent-01",
                    MatchType::Exact,
                    MatchType::Exact,
                )
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        },
    };
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_user_and_agent_rule_user_mismatch() {
    let ruleset = RuleSetBuilder::new()
        .version("2.0")
        .rule(
            RuleBuilder::new()
                .name("alice-read")
                .subject_user_and_agent(
                    "ou_alice",
                    "dev-agent-01",
                    MatchType::Exact,
                    MatchType::Exact,
                )
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_bob".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        },
    };
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

#[tokio::test]
async fn test_bare_request_uses_agent_only_matching() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("dev-agent-full")
                .subject_agent("dev-agent-01")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "dev-agent-01".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    });
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

#[tokio::test]
async fn test_with_caller_request_still_matches_agent_only_rules() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("dev-agent-full")
                .subject_agent("dev-agent-01")
                .allow()
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);

    let request = PermissionRequest::WithCaller {
        caller: Caller {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            creator_id: String::new(),
        },
        request: PermissionRequestBody::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        },
    };
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}

// =============================================================================
// Rule::validate() tests
// =============================================================================

#[test]
fn test_validate_actions_only() {
    let rule = Rule {
        name: "test".to_string(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: "read".to_string(),
            paths: vec!["**".to_string()],
        }],
        template: None,
        priority: 0,
    };
    assert!(rule.validate().is_ok());
}

#[test]
fn test_validate_template_only() {
    let rule = Rule {
        name: "test".to_string(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
        template: Some(TemplateRef {
            name: "developer".to_string(),
            overrides: Default::default(),
        }),
        priority: 0,
    };
    assert!(rule.validate().is_ok());
}

#[test]
fn test_validate_actions_and_template_mutually_exclusive() {
    let rule = Rule {
        name: "test".to_string(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: "read".to_string(),
            paths: vec!["**".to_string()],
        }],
        template: Some(TemplateRef {
            name: "developer".to_string(),
            overrides: Default::default(),
        }),
        priority: 0,
    };
    let err = rule.validate().unwrap_err();
    assert!(err.contains("mutually exclusive"));
}

#[test]
fn test_validate_at_least_one_required() {
    let rule = Rule {
        name: "test".to_string(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
        template: None,
        priority: 0,
    };
    let err = rule.validate().unwrap_err();
    assert!(err.contains("at least one"));
}

// =============================================================================
// Subject JSON deserialization tests
// =============================================================================

#[test]
fn test_subject_deserialize_old_agent_only() {
    let json = r#"{"agent": "dev-agent-01", "match_type": "exact"}"#;
    let subject: Subject = serde_json::from_str(json).unwrap();
    assert!(matches!(subject, Subject::AgentOnly { .. }));
    assert_eq!(subject.agent_id(), "dev-agent-01");
}

#[test]
fn test_subject_deserialize_old_with_glob() {
    let json = r#"{"agent": "dev-*", "match": "glob"}"#;
    let subject: Subject = serde_json::from_str(json).unwrap();
    assert!(
        matches!(subject, Subject::AgentOnly { agent, match_type: MatchType::Glob } if agent == "dev-*")
    );
}

#[test]
fn test_subject_deserialize_new_user_and_agent() {
    let json = r#"{
        "match_mode": "user_and_agent",
        "user_id": "ou_alice",
        "agent": "dev-agent-01",
        "user_match": "exact",
        "agent_match": "exact"
    }"#;
    let subject: Subject = serde_json::from_str(json).unwrap();
    let Subject::UserAndAgent { user_id, agent, .. } = subject else {
        panic!("expected UserAndAgent")
    };
    assert_eq!(user_id, "ou_alice");
    assert_eq!(agent, "dev-agent-01");
}

// =============================================================================
// Template tests
// =============================================================================

use std::collections::HashMap;

#[test]
fn test_template_deserialize() {
    let json = r#"{
        "name": "developer",
        "description": "Standard development permissions",
        "subject": { "type": "agent", "agent": "dev-*", "match_type": "glob" },
        "effect": "allow",
        "actions": [
            { "type": "file", "operation": "read", "paths": ["**"] },
            { "type": "command", "command": "cargo" }
        ],
        "extends": []
    }"#;
    let tmpl: Template = serde_json::from_str(json).unwrap();
    assert_eq!(tmpl.name, "developer");
    assert!(matches!(tmpl.subject, TemplateSubject::Agent { .. }));
    assert_eq!(tmpl.actions.len(), 2);
}

#[test]
fn test_template_subject_any() {
    let json = r#"{
        "name": "readonly",
        "subject": { "type": "any" },
        "actions": [{ "type": "file", "operation": "read", "paths": ["**"] }]
    }"#;
    let tmpl: Template = serde_json::from_str(json).unwrap();
    assert!(matches!(tmpl.subject, TemplateSubject::Any));
}

#[test]
fn test_template_subject_user_and_agent() {
    let json = r#"{
        "name": "user-dev",
        "subject": {
            "type": "user_and_agent",
            "user_id": "ou_123",
            "agent": "dev-*",
            "user_match": "exact",
            "agent_match": "glob"
        },
        "actions": [{ "type": "file", "operation": "read", "paths": ["**"] }]
    }"#;
    let tmpl: Template = serde_json::from_str(json).unwrap();
    assert!(matches!(tmpl.subject, TemplateSubject::UserAndAgent { .. }));
}

#[test]
fn test_template_inheritance_expansion() {
    use crate::permission::templates::expand_inheritance;

    let mut templates: HashMap<String, Template> = HashMap::new();
    templates.insert(
        "base".to_string(),
        Template {
            name: "base".to_string(),
            description: "".to_string(),
            subject: TemplateSubject::Any,
            effect: Effect::Allow,
            actions: vec![Action::File {
                operation: "read".to_string(),
                paths: vec!["**".to_string()],
            }],
            extends: vec![],
        },
    );
    templates.insert(
        "extended".to_string(),
        Template {
            name: "extended".to_string(),
            description: "".to_string(),
            subject: TemplateSubject::Any,
            effect: Effect::Allow,
            actions: vec![Action::Command {
                command: "git".to_string(),
                args: crate::permission::engine::CommandArgs::Any,
            }],
            extends: vec!["base".to_string()],
        },
    );

    expand_inheritance(&mut templates).unwrap();

    let extended = templates.get("extended").unwrap();
    assert!(extended.actions.len() >= 2);
}

#[test]
fn test_template_cycle_detection() {
    use crate::permission::templates::expand_inheritance;

    let mut templates: HashMap<String, Template> = HashMap::new();
    templates.insert(
        "a".to_string(),
        Template {
            name: "a".to_string(),
            description: "".to_string(),
            subject: TemplateSubject::Any,
            effect: Effect::Allow,
            actions: vec![],
            extends: vec!["b".to_string()],
        },
    );
    templates.insert(
        "b".to_string(),
        Template {
            name: "b".to_string(),
            description: "".to_string(),
            subject: TemplateSubject::Any,
            effect: Effect::Allow,
            actions: vec![],
            extends: vec!["a".to_string()],
        },
    );

    let result = expand_inheritance(&mut templates);
    assert!(result.is_err());
}

// =============================================================================
// Priority tests
// =============================================================================

#[tokio::test]
async fn test_rule_priority_higher_evaluated_first() {
    let ruleset = RuleSetBuilder::new()
        .version("1.0")
        .rule(
            RuleBuilder::new()
                .name("low-priority-allow")
                .subject_agent("test-agent")
                .allow()
                .priority(0)
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("high-priority-deny")
                .subject_agent("test-agent")
                .deny()
                .priority(10)
                .action(Action::File {
                    operation: "read".to_string(),
                    paths: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new(ruleset);
    let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/any/path.txt".to_string(),
        op: "read".to_string(),
    });
    let response = engine.evaluate(request);
    assert!(matches!(response, PermissionResponse::Denied { .. }));
}

// =============================================================================
// Action::All tests
// =============================================================================

#[test]
fn test_action_all_matches_any_request() {
    let action = Action::All;
    let requests = [
        PermissionRequestBody::FileOp {
            agent: "test".into(),
            path: "/".into(),
            op: "read".into(),
        },
        PermissionRequestBody::CommandExec {
            agent: "test".into(),
            cmd: "rm".into(),
            args: vec![],
        },
        PermissionRequestBody::NetOp {
            agent: "test".into(),
            host: "evil.com".into(),
            port: 80,
        },
        PermissionRequestBody::ConfigWrite {
            agent: "test".into(),
            config_file: "/etc/passwd".into(),
        },
    ];
    for req in requests {
        assert!(
            crate::permission::engine::action_matches_request(&action, &req),
            "Action::All should match {:?}",
            req
        );
    }
}

// =============================================================================
// Validation integration tests
// =============================================================================

#[test]
fn test_validation_user_and_agent_subject() {
    let rule = Rule {
        name: "test".to_string(),
        subject: Subject::UserAndAgent {
            user_id: "ou_alice".to_string(),
            agent: "dev-agent-01".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: "read".to_string(),
            paths: vec!["**".to_string()],
        }],
        template: None,
        priority: 0,
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors.is_empty(), "expected no errors, got {:?}", errors);
}

#[test]
fn test_validation_with_template() {
    let rule = Rule {
        name: "test".to_string(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
        template: Some(TemplateRef {
            name: "developer".to_string(),
            overrides: Default::default(),
        }),
        priority: 0,
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors.is_empty(), "expected no errors, got {:?}", errors);
}
