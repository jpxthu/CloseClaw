//!
//! Rule evaluation tests (User+Agent matching, priority)
//!

use crate::permission::engine::{
    Action, Caller, Effect, MatchType, PermissionEngine, PermissionRequest, PermissionRequestBody,
    PermissionResponse,
};
use crate::permission::rules::{RuleBuilder, RuleSetBuilder};

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
