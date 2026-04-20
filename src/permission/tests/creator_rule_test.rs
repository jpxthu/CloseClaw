//!
//! Creator Rule short-circuit tests
//!

use crate::permission::engine::{
    Action, Caller, Effect, MatchType, PermissionEngine, PermissionRequest, PermissionRequestBody,
    PermissionResponse,
};
use crate::permission::rules::RuleSetBuilder;

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
            crate::permission::rules::RuleBuilder::new()
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
    assert!(matches!(response, PermissionResponse::Allowed { .. }));
}
