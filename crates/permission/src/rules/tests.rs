use super::*;
use crate::actions::ActionBuilder;
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
        .default_file(Effect::Deny)
        .build()
        .unwrap();

    assert_eq!(ruleset.rules.len(), 1);
    assert_eq!(ruleset.defaults.file, Effect::Deny);
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
    assert_eq!(defaults.file, Effect::Deny);
    assert_eq!(defaults.tool_call, Effect::Deny);
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
