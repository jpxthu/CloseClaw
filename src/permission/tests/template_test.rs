//!
//! Template tests (deserialization, inheritance, cycle detection)
//!

use std::collections::HashMap;

use crate::permission::engine::{Action, Effect};
use crate::permission::templates::{expand_inheritance, Template, TemplateSubject};

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
