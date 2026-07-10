use super::*;
use crate::actions::ActionBuilder;
use crate::engine::engine_types::{PermissionRequest, PermissionRequestBody, PermissionResponse};
use serde_json;

#[test]
fn test_rule_builder() {
    let rule = RuleBuilder::new()
        .name("allow-read-home")
        .subject_agent("dev-agent-01")
        .allow()
        .action(
            ActionBuilder::file("read", vec!["/home/**".to_string()])
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();

    assert_eq!(rule.name, "allow-read-home");
    assert_eq!(rule.subject.agent_id(), "dev-agent-01");
    assert!(matches!(rule.effect, Effect::Allow));
    assert_eq!(rule.actions.len(), 1);
}

#[test]
fn test_rule_builder_missing_name() {
    let result = RuleBuilder::new().subject_agent("dev-agent-01").build();

    assert!(matches!(
        result,
        Err(RuleBuilderError::MissingField("name"))
    ));
}

#[test]
fn test_ruleset_builder() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("test-rule")
                .subject_agent("test-agent")
                .allow()
                .build()
                .unwrap(),
        )
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .build()
        .unwrap();

    assert_eq!(ruleset.rules.len(), 1);
    assert_eq!(ruleset.defaults.file_read, Effect::Deny);
    assert_eq!(ruleset.defaults.file_write, Effect::Deny);
}

#[test]
fn test_validation() {
    let empty_rule = Rule {
        name: String::new(),
        subject: Subject::AgentOnly {
            agent: String::new(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
        template: None,
        priority: 0,
    };

    let errors = validation::validate_rule(&empty_rule);
    assert!(errors
        .iter()
        .any(|e| matches!(e, validation::RuleValidationError::EmptyName)));
    assert!(errors
        .iter()
        .any(|e| matches!(e, validation::RuleValidationError::EmptySubjectAgent)));
    assert!(errors
        .iter()
        .any(|e| matches!(e, validation::RuleValidationError::NoActions)));
}

// Additional validation tests (from comprehensive_tests.rs)
#[test]
fn test_validation_empty_rule_name() {
    let rule = Rule {
        name: String::new(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
        template: None,
        priority: 0,
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors
        .iter()
        .any(|e| matches!(e, validation::RuleValidationError::EmptyName)));
}

#[test]
fn test_validation_empty_subject_agent() {
    let rule = Rule {
        name: "test-rule".to_string(),
        subject: Subject::AgentOnly {
            agent: String::new(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
        template: None,
        priority: 0,
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors
        .iter()
        .any(|e| matches!(e, validation::RuleValidationError::EmptySubjectAgent)));
}

#[test]
fn test_validation_no_actions() {
    let rule = Rule {
        name: "test-rule".to_string(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Allow,
        actions: vec![],
        template: None,
        priority: 0,
    };
    let errors = validation::validate_rule(&rule);
    assert!(errors
        .iter()
        .any(|e| matches!(e, validation::RuleValidationError::NoActions)));
}

#[test]
fn test_validation_has_deny_rules() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("deny-rule")
                .subject_agent("test")
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
    assert!(validation::has_deny_rules(&ruleset));
    assert!(!validation::has_allow_rules(&ruleset));
}

#[test]
fn test_validation_has_allow_rules() {
    let ruleset = RuleSetBuilder::new()
        .rule(
            RuleBuilder::new()
                .name("allow-rule")
                .subject_agent("test")
                .allow()
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
    assert!(!validation::has_deny_rules(&ruleset));
    assert!(validation::has_allow_rules(&ruleset));
}

#[test]
fn test_defaults_tool_call_is_deny() {
    let defaults = Defaults::default();
    assert_eq!(defaults.tool_call, Effect::Deny);
}

#[test]
fn test_defaults_json_missing_tool_call() {
    let json = r#"{"file":"allow","command":"deny","network":"deny","inter_agent":"deny","config":"deny"}"#;
    let defaults: Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(defaults.tool_call, Effect::Deny);
}

#[test]
fn test_defaults_json_missing_message_defaults_to_allow() {
    // Old config without `message` field should default to Allow
    let json = r#"{"file":"deny","command":"deny","network":"deny","inter_agent":"deny","config":"deny","tool_call":"deny"}"#;
    let defaults: Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(defaults.message, Effect::Allow);
}

#[test]
fn test_defaults_json_empty_object_message_is_allow() {
    let json = r#"{}"#;
    let defaults: Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(defaults.message, Effect::Allow);
    assert_eq!(defaults.file_read, Effect::Deny);
    assert_eq!(defaults.file_write, Effect::Deny);
    assert_eq!(defaults.tool_call, Effect::Deny);
}

// ------------------------------------------------------------------
// Step 1.3: Independent file_read / file_write defaults
// ------------------------------------------------------------------

#[test]
fn test_file_read_and_file_write_independent_defaults() {
    // file_read = Allow, file_write = Deny
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .build()
        .unwrap();
    let engine = crate::engine::PermissionEngine::new_with_default_data_root(ruleset);

    // FileOp read → file_read default (Allow)
    let read_resp = engine.check("unknown-agent", "file_read", None);
    assert!(
        matches!(read_resp, PermissionResponse::Allowed { .. }),
        "file_read default Allow should produce Allowed, got {:?}",
        read_resp
    );

    // FileOp write → file_write default (Deny)
    let write_resp = engine.check("unknown-agent", "file_write", None);
    assert!(
        matches!(write_resp, PermissionResponse::Denied { .. }),
        "file_write default Deny should produce Denied, got {:?}",
        write_resp
    );
}

#[test]
fn test_file_read_deny_file_write_allow_independent() {
    // file_read = Deny, file_write = Allow (reverse of above)
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .build()
        .unwrap();
    let engine = crate::engine::PermissionEngine::new_with_default_data_root(ruleset);

    let read_resp = engine.check("unknown-agent", "file_read", None);
    assert!(
        matches!(read_resp, PermissionResponse::Denied { .. }),
        "file_read default Deny should produce Denied, got {:?}",
        read_resp
    );

    let write_resp = engine.check("unknown-agent", "file_write", None);
    assert!(
        matches!(write_resp, PermissionResponse::Allowed { .. }),
        "file_write default Allow should produce Allowed, got {:?}",
        write_resp
    );
}

#[test]
fn test_file_read_write_default_response_dispatches_correctly() {
    // Verify that default_response() dispatches based on the FileOp op field:
    // read → file_read default, write → file_write default.
    // Use evaluate() with explicit FileOp requests to be precise.
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .build()
        .unwrap();
    let engine = crate::engine::PermissionEngine::new_with_default_data_root(ruleset);

    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("test.txt").to_string_lossy().into_owned();

    // FileOp read → should use file_read default (Allow)
    let read_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "unknown-agent".to_string(),
            path: path.clone(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(
        matches!(read_resp, PermissionResponse::Allowed { .. }),
        "FileOp read should use file_read default (Allow), got {:?}",
        read_resp
    );

    // FileOp write → should use file_write default (Deny)
    let write_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "unknown-agent".to_string(),
            path: path.clone(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(write_resp, PermissionResponse::Denied { .. }),
        "FileOp write should use file_write default (Deny), got {:?}",
        write_resp
    );
}

// ------------------------------------------------------------------
// Step 1.3: Old config 'file' field deserialization compat
// ------------------------------------------------------------------

#[test]
fn test_old_config_file_field_sets_both_read_and_write() {
    // Old config with single 'file' field should set both file_read and file_write.
    let json = r#"{"file": "allow", "command": "deny", "network": "deny"}"#;
    let defaults: Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(
        defaults.file_read,
        Effect::Allow,
        "old 'file' field should set file_read"
    );
    assert_eq!(
        defaults.file_write,
        Effect::Allow,
        "old 'file' field should set file_write"
    );
}

#[test]
fn test_explicit_file_read_write_overrides_old_file() {
    // When both 'file' and 'file_read'/'file_write' are present,
    // explicit fields take precedence.
    let json = r#"{"file": "allow", "file_read": "deny", "file_write": "allow"}"#;
    let defaults: Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(
        defaults.file_read,
        Effect::Deny,
        "file_read should override old 'file' field"
    );
    assert_eq!(
        defaults.file_write,
        Effect::Allow,
        "file_write should override old 'file' field"
    );
}

#[test]
fn test_ruleset_builder_default_tool_call() {
    let ruleset = RuleSetBuilder::new()
        .default_tool_call(Effect::Allow)
        .build()
        .unwrap();
    assert_eq!(ruleset.defaults.tool_call, Effect::Allow);
}

#[test]
fn test_rule_builder_missing_subject() {
    let result = RuleBuilder::new()
        .name("test-rule")
        .allow()
        .action(
            ActionBuilder::file("read", vec!["**".to_string()])
                .build()
                .unwrap(),
        )
        .build();
    assert!(matches!(
        result,
        Err(RuleBuilderError::MissingField("subject"))
    ));
}
