//! ConfigWrite guard tests — verify that ConfigWrite Allow rules are
//! forcibly overridden to Denied by the hardcoded guard.

use crate::engine::{
    Action, Effect, PermissionEngine, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use crate::rules::{RuleBuilder, RuleSetBuilder};

/// ConfigWrite Allow rule is forcibly overridden to Denied.
#[test]
fn test_config_write_allow_rule_blocked() {
    let ruleset = RuleSetBuilder::new()
        .default_config(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("allow-config-write")
                .subject_agent("test-agent")
                .allow()
                .action(Action::ConfigWrite {
                    files: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
        agent: "test-agent".to_string(),
        config_file: "config.yaml".to_string(),
    });
    let response = engine.evaluate(request, None);

    assert!(
        matches!(response, PermissionResponse::Denied { .. }),
        "ConfigWrite Allow rule should be forced to Denied, got {:?}",
        response
    );
}

/// ConfigWrite Deny rule still works as expected.
#[test]
fn test_config_write_deny_rule_still_works() {
    let ruleset = RuleSetBuilder::new()
        .default_config(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("deny-config-write")
                .subject_agent("test-agent")
                .deny()
                .action(Action::ConfigWrite {
                    files: vec!["**".to_string()],
                })
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
        agent: "test-agent".to_string(),
        config_file: "config.yaml".to_string(),
    });
    let response = engine.evaluate(request, None);

    assert!(
        matches!(response, PermissionResponse::Denied { .. }),
        "ConfigWrite Deny rule should return Denied, got {:?}",
        response
    );
}

/// ConfigWrite with default Allow is now intercepted by the default guard.
/// Design doc: "此维度永远高危，只能走单次审批".
#[test]
fn test_config_write_default_allow_not_intercepted_by_guard() {
    let ruleset = RuleSetBuilder::new()
        .default_config(Effect::Allow)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
        agent: "test-agent".to_string(),
        config_file: "config.yaml".to_string(),
    });
    let response = engine.evaluate(request, None);

    // Step 1.8: default guard now intercepts ConfigWrite even when
    // defaults.config is Allow, because this dimension is always high-risk.
    assert!(
        matches!(response, PermissionResponse::Denied { .. }),
        "ConfigWrite default Allow should be intercepted by default guard, got {:?}",
        response
    );
}

/// ConfigWrite with default Deny returns Denied (normal path).
#[test]
fn test_config_write_default_deny() {
    let ruleset = RuleSetBuilder::new()
        .default_config(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
        agent: "test-agent".to_string(),
        config_file: "config.yaml".to_string(),
    });
    let response = engine.evaluate(request, None);

    assert!(
        matches!(response, PermissionResponse::Denied { .. }),
        "ConfigWrite default Deny should return Denied, got {:?}",
        response
    );
}

/// MessageSend direction is correctly passed through check().
#[test]
fn test_check_message_action() {
    let ruleset = RuleSetBuilder::new()
        .default_message(Effect::Allow)
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .build()
        .unwrap();

    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let response = engine.check("test-agent", "message", None);

    assert!(
        matches!(response, PermissionResponse::Allowed { .. }),
        "check('agent', 'message', None) should not return unknown action, got {:?}",
        response
    );
}
