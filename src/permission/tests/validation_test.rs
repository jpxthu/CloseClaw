use crate::permission::engine::{
    Action, MatchType, PermissionRequest, PermissionRequestBody, Rule, Subject, TemplateRef,
};

#[test]
fn test_validate_actions_only() {
    let rule = Rule {
        name: "test".to_string(),
        subject: Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        },
        effect: crate::permission::engine::Effect::Allow,
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
        effect: crate::permission::engine::Effect::Allow,
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
        effect: crate::permission::engine::Effect::Allow,
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
        effect: crate::permission::engine::Effect::Allow,
        actions: vec![],
        template: None,
        priority: 0,
    };
    let err = rule.validate().unwrap_err();
    assert!(err.contains("at least one"));
}

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
        effect: crate::permission::engine::Effect::Allow,
        actions: vec![Action::File {
            operation: "read".to_string(),
            paths: vec!["**".to_string()],
        }],
        template: None,
        priority: 0,
    };
    let errors = crate::permission::rules::validation::validate_rule(&rule);
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
        effect: crate::permission::engine::Effect::Allow,
        actions: vec![],
        template: Some(TemplateRef {
            name: "developer".to_string(),
            overrides: Default::default(),
        }),
        priority: 0,
    };
    let errors = crate::permission::rules::validation::validate_rule(&rule);
    assert!(errors.is_empty(), "expected no errors, got {:?}", errors);
}
