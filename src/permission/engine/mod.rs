//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

pub mod engine_eval;
pub mod engine_matching;
pub mod engine_types;

pub use engine_eval::PermissionEngine;
pub use engine_matching::{action_matches_request, glob_match};
pub use engine_types::{
    Action, Caller, CommandArgs, Defaults, Effect, MatchType, PermissionRequest,
    PermissionRequestBody, PermissionResponse, Rule, RuleSet, Subject, TemplateRef,
};

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::engine_eval::PermissionEngine;
    use super::engine_matching::glob_match;
    use super::engine_types::{
        Action, Caller, CommandArgs, Effect, MatchType, PermissionRequest, PermissionRequestBody,
        PermissionResponse, Rule, RuleSet, Subject,
    };
    use crate::permission::actions::ActionBuilder;
    use crate::permission::rules::{RuleBuilder, RuleSetBuilder};

    // -------------------------------------------------------------------------
    // Glob matching tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_glob_exact() {
        assert!(glob_match("dev-agent-01", "dev-agent-01"));
        assert!(!glob_match("dev-agent-01", "dev-agent-02"));
    }

    #[test]
    fn test_glob_star() {
        assert!(glob_match("readonly-*", "readonly-agent-1"));
        assert!(glob_match("readonly-*", "readonly-agent-42"));
        assert!(!glob_match("readonly-*", "readonly"));
    }

    #[test]
    fn test_glob_question() {
        assert!(glob_match("file_?.txt", "file_a.txt"));
        assert!(glob_match("file_?.txt", "file_1.txt"));
        assert!(!glob_match("file_?.txt", "file_12.txt"));
    }

    #[test]
    fn test_glob_double_star() {
        assert!(glob_match(
            "/home/admin/code/**",
            "/home/admin/code/closeclaw/src/main.rs"
        ));
        assert!(glob_match(
            "/home/admin/code/**",
            "/home/admin/code/closeclaw/src/permission/engine.rs"
        ));
        assert!(!glob_match("/home/admin/code/**", "/home/admin/other/path"));
    }

    // -------------------------------------------------------------------------
    // PermissionEngine basic tests
    // -------------------------------------------------------------------------

    fn make_engine() -> PermissionEngine {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Deny)
            .default_command(Effect::Deny)
            .default_network(Effect::Deny)
            .default_inter_agent(Effect::Deny)
            .default_config(Effect::Deny)
            .rule(
                RuleBuilder::new()
                    .name("allow-read")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::file("read", vec!["/data/**".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .rule(
                RuleBuilder::new()
                    .name("deny-write")
                    .subject_agent("test-agent")
                    .deny()
                    .action(
                        ActionBuilder::file("write", vec!["/etc/**".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        PermissionEngine::new(ruleset)
    }

    #[test]
    fn test_engine_allow_read() {
        let engine = make_engine();
        let resp = engine.check("test-agent", "file_read");
        matches!(resp, PermissionResponse::Allowed { .. });
    }

    #[test]
    fn test_engine_default_deny() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Deny)
            .default_command(Effect::Deny)
            .default_network(Effect::Deny)
            .default_inter_agent(Effect::Deny)
            .default_config(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let resp = engine.check("unknown-agent", "file_read");
        matches!(resp, PermissionResponse::Denied { .. });
    }

    #[test]
    fn test_deny_takes_precedence() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Allow)
            .rule(
                RuleBuilder::new()
                    .name("deny-sensitive")
                    .subject_agent("test-agent")
                    .deny()
                    .action(
                        ActionBuilder::file("write", vec!["/etc/shadow".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let resp = engine.evaluate(PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/etc/shadow".to_string(),
            op: "write".to_string(),
        }));
        matches!(resp, PermissionResponse::Denied { .. });
    }

    #[test]
    fn test_subject_agent_only_exact() {
        let subject = Subject::AgentOnly {
            agent: "dev-agent-01".to_string(),
            match_type: MatchType::Exact,
        };
        let caller = Caller {
            agent: "dev-agent-01".to_string(),
            ..Default::default()
        };
        assert!(subject.matches(&caller));
        let caller2 = Caller {
            agent: "dev-agent-02".to_string(),
            ..Default::default()
        };
        assert!(!subject.matches(&caller2));
    }

    #[test]
    fn test_subject_agent_only_glob() {
        let subject = Subject::AgentOnly {
            agent: "test-*".to_string(),
            match_type: MatchType::Glob,
        };
        let caller = Caller {
            agent: "test-agent".to_string(),
            ..Default::default()
        };
        assert!(subject.matches(&caller));
    }

    #[test]
    fn test_subject_user_and_agent() {
        let subject = Subject::UserAndAgent {
            user_id: "alice".to_string(),
            agent: "test-agent".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        };
        let caller = Caller {
            user_id: "alice".to_string(),
            agent: "test-agent".to_string(),
            ..Default::default()
        };
        assert!(subject.matches(&caller));

        let caller_wrong_user = Caller {
            user_id: "bob".to_string(),
            agent: "test-agent".to_string(),
            ..Default::default()
        };
        assert!(!subject.matches(&caller_wrong_user));
    }

    #[test]
    fn test_rule_validate_ok() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::All],
            template: None,
            priority: 0,
        };
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn test_rule_validate_neither() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![],
            template: None,
            priority: 0,
        };
        assert!(rule.validate().is_err());
    }

    #[test]
    fn test_permission_request_bare() {
        let req = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        });
        let caller = req.caller();
        assert_eq!(caller.agent, "test-agent");
        assert!(caller.user_id.is_empty());
    }

    #[test]
    fn test_permission_request_with_caller() {
        let req = PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::CommandExec {
                agent: "test-agent".to_string(),
                cmd: "ls".to_string(),
                args: vec![],
            },
        };
        let caller = req.caller();
        assert_eq!(caller.user_id, "alice");
        assert_eq!(caller.agent, "test-agent");
    }

    #[test]
    fn test_rule_parse_subject() {
        let subject = Rule::parse_subject("test-agent");
        matches!(subject, Subject::AgentOnly { agent, match_type: MatchType::Exact }
            if agent == "test-agent");
    }

    #[test]
    fn test_subject_is_agent_only() {
        let agent_only = Subject::AgentOnly {
            agent: "test".to_string(),
            match_type: MatchType::Exact,
        };
        assert!(agent_only.is_agent_only());
        let ua = Subject::UserAndAgent {
            user_id: "u".to_string(),
            agent: "a".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Exact,
        };
        assert!(!ua.is_agent_only());
    }

    #[test]
    fn test_caller_defaults() {
        let caller = Caller::default();
        assert!(caller.user_id.is_empty());
        assert!(caller.agent.is_empty());
    }

    #[test]
    fn test_permission_request_body_agent_id() {
        let req = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "echo".to_string(),
            args: vec![],
        });
        assert_eq!(req.body().agent_id(), "test-agent");
    }

    #[test]
    fn test_command_args_any() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![],
            template: None,
            priority: 0,
        };
        let args = CommandArgs::Any;
        assert!(rule.args_match(&args, &["anything".to_string()]));
    }

    #[test]
    fn test_command_args_allowed() {
        let rule = Rule {
            name: "test".to_string(),
            subject: Subject::AgentOnly {
                agent: "agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![],
            template: None,
            priority: 0,
        };
        let args_allowed = CommandArgs::Allowed {
            allowed: vec!["--foo".to_string(), "--bar".to_string()],
        };
        assert!(rule.args_match(&args_allowed, &["--foo".to_string()]));
        assert!(rule.args_match(&args_allowed, &["--bar".to_string()]));
        assert!(!rule.args_match(&args_allowed, &["--baz".to_string()]));
    }
}
