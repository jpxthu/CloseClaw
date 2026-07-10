use tempfile::TempDir;

use super::engine_eval::PermissionEngine;
use super::engine_matching::glob_match;
use super::engine_types::{
    Action, Caller, CommandArgs, Effect, MatchType, PermissionRequest, PermissionRequestBody,
    PermissionResponse, Rule, Subject,
};
use crate::actions::ActionBuilder;
use crate::rules::{RuleBuilder, RuleSetBuilder};

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
    PermissionEngine::new_with_default_data_root(ruleset)
}

#[test]
fn test_engine_allow_read() {
    let engine = make_engine();
    let resp = engine.check("test-agent", "file_read", None);
    matches!(resp, PermissionResponse::Allowed { .. });
}

#[test]
fn test_engine_default_deny() {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let resp = engine.check("unknown-agent", "file_read", None);
    matches!(resp, PermissionResponse::Denied { .. });
}

#[test]
fn test_deny_takes_precedence() {
    let ruleset = RuleSetBuilder::new()
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
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/etc/shadow".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
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

#[test]
fn test_tool_call_default_independent_from_file() {
    // tool_call defaults to Deny while file defaults to Allow
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_tool_call(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // file op should be allowed (default Allow)
    let tmp = TempDir::new().unwrap();
    let file_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "unknown-agent".to_string(),
            path: tmp.path().to_string_lossy().into_owned(),
            op: "read".to_string(),
        }),
        None,
    );
    matches!(file_resp, PermissionResponse::Allowed { .. });

    // tool call should be denied (default Deny)
    let tool_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "unknown-agent".to_string(),
            skill: "some_tool".to_string(),
            method: "run".to_string(),
        }),
        None,
    );
    matches!(tool_resp, PermissionResponse::Denied { .. });
}

#[test]
fn test_tool_call_default_allow() {
    let ruleset = RuleSetBuilder::new()
        .default_tool_call(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let resp = engine.check("unknown-agent", "tool_call", None);
    matches!(resp, PermissionResponse::Allowed { .. });
}

// -----------------------------------------------------------------
// MessageSend default Allow tests (design doc: Agent defaults to
// send/receive messages)
// -----------------------------------------------------------------

#[test]
fn test_message_send_defaults_to_allow() {
    // With all defaults at Deny except message=Allow (the design-doc contract),
    // MessageSend should be Allowed while other operations are Denied.
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .default_message(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    // MessageSend should be Allowed by default
    let msg_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::MessageSend {
            agent: "unknown-agent".to_string(),
            direction: super::engine_types::MessageDirection::Send,
            target: "chat_1".to_string(),
        }),
        None,
    );
    assert!(
        matches!(msg_resp, PermissionResponse::Allowed { .. }),
        "MessageSend should default to Allow, got {:?}",
        msg_resp
    );
}

/// Helper: build a ruleset with all dimensions Deny except message=Allow.
fn deny_all_except_message_ruleset() -> super::engine_types::RuleSet {
    RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .default_message(Effect::Allow)
        .build()
        .unwrap()
}

#[test]
fn test_file_op_defaults_to_deny() {
    let ruleset = deny_all_except_message_ruleset();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let tmp = TempDir::new().unwrap();
    let file_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "unknown-agent".to_string(),
            path: tmp.path().to_string_lossy().into_owned(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(
        matches!(file_resp, PermissionResponse::Denied { .. }),
        "FileOp should default to Deny, got {:?}",
        file_resp
    );
}

#[test]
fn test_command_exec_defaults_to_deny() {
    let ruleset = deny_all_except_message_ruleset();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let cmd_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "unknown-agent".to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        }),
        None,
    );
    assert!(
        matches!(cmd_resp, PermissionResponse::Denied { .. }),
        "CommandExec should default to Deny, got {:?}",
        cmd_resp
    );
}

#[test]
fn test_net_op_defaults_to_deny() {
    let ruleset = deny_all_except_message_ruleset();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let net_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::NetOp {
            agent: "unknown-agent".to_string(),
            host: "example.com".to_string(),
            port: 443,
        }),
        None,
    );
    assert!(
        matches!(net_resp, PermissionResponse::Denied { .. }),
        "NetOp should default to Deny, got {:?}",
        net_resp
    );
}

#[test]
fn test_tool_call_defaults_to_deny() {
    let ruleset = deny_all_except_message_ruleset();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let tool_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ToolCall {
            agent: "unknown-agent".to_string(),
            skill: "some_skill".to_string(),
            method: "run".to_string(),
        }),
        None,
    );
    assert!(
        matches!(tool_resp, PermissionResponse::Denied { .. }),
        "ToolCall should default to Deny, got {:?}",
        tool_resp
    );
}

#[test]
fn test_message_send_default_can_be_overridden_to_deny() {
    // Verify that the message default can be explicitly set to Deny.
    let ruleset = RuleSetBuilder::new()
        .default_message(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);

    let msg_resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::MessageSend {
            agent: "unknown-agent".to_string(),
            direction: super::engine_types::MessageDirection::Send,
            target: "chat_1".to_string(),
        }),
        None,
    );
    assert!(
        matches!(msg_resp, PermissionResponse::Denied { .. }),
        "MessageSend with default Deny should be Denied, got {:?}",
        msg_resp
    );
}

// -----------------------------------------------------------------
// Config dir forced-deny integration tests
// -----------------------------------------------------------------

#[test]
fn test_config_dir_forced_deny_overrides_default_allow() {
    // When default_file = Allow, a FileOp targeting the config directory
    // (data_root itself) must still be denied by the hardcoded guard.
    let tmp = TempDir::new().unwrap();
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // Read from config dir (agents/) → should be denied
    let config_path = tmp
        .path()
        .join("agents/a1/permissions.json")
        .to_string_lossy()
        .into_owned();
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: config_path,
        op: "read".to_string(),
    });
    let resp = engine.evaluate(req, None);
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "<config_dir_guard>"),
        "expected Denied from config_dir_guard, got {:?}",
        resp
    );
}

#[test]
fn test_config_dir_forced_deny_write() {
    // Write to config dir → should also be denied.
    let tmp = TempDir::new().unwrap();
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    let config_path = tmp
        .path()
        .join("agents/a1/permissions.json")
        .to_string_lossy()
        .into_owned();
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: config_path,
        op: "write".to_string(),
    });
    let resp = engine.evaluate(req, None);
    assert!(
        matches!(resp, PermissionResponse::Denied { ref rule, .. } if rule == "<config_dir_guard>"),
        "expected Denied from config_dir_guard for write, got {:?}",
        resp
    );
}

#[test]
fn test_workspace_still_allowed_with_default_allow() {
    // When default_file = Allow, workspace FileOp should still be allowed.
    let tmp = TempDir::new().unwrap();
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    let ws = tmp
        .path()
        .join("workspaces/test-agent/test-user/file.txt")
        .to_string_lossy()
        .into_owned();
    let req = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: ws,
        op: "read".to_string(),
    });
    let resp = engine.evaluate(req, None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "expected Allowed for workspace read, got {:?}",
        resp
    );
}

// ----------------------------------------------------------------------
// Rule version computation on engine construction
// ----------------------------------------------------------------------

#[test]
fn test_engine_new_computes_rule_version() {
    // PermissionEngine::new() should compute rule_version from the ruleset.
    let tmp = TempDir::new().unwrap();
    let ruleset = RuleSetBuilder::new().build().unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());
    let version = engine.rules().rule_version.clone();
    assert!(
        !version.is_empty(),
        "rule_version should be computed on new()"
    );
}

#[test]
fn test_engine_reload_computes_rule_version() {
    // PermissionEngine::reload_rules() should recompute rule_version.
    let tmp = TempDir::new().unwrap();
    let ruleset_a = RuleSetBuilder::new().build().unwrap();
    let mut engine = PermissionEngine::new(ruleset_a, tmp.path().to_path_buf());
    let version_a = engine.rules().rule_version.clone();

    let ruleset_b = RuleSetBuilder::new().build().unwrap();
    engine.reload_rules(ruleset_b);
    let version_b = engine.rules().rule_version.clone();

    assert!(
        !version_b.is_empty(),
        "rule_version should be computed on reload_rules()"
    );
    // Same defaults → same version
    assert_eq!(
        version_a, version_b,
        "identical rulesets should produce same version"
    );
}

#[test]
fn test_engine_new_different_rules_different_version() {
    // Two engines with different rules should have different versions.
    let tmp = TempDir::new().unwrap();
    let ruleset_a = RuleSetBuilder::new().build().unwrap();
    let engine_a = PermissionEngine::new(ruleset_a, tmp.path().to_path_buf());

    let ruleset_b = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .build()
        .unwrap();
    let engine_b = PermissionEngine::new(ruleset_b, tmp.path().to_path_buf());

    assert_ne!(
        engine_a.rules().rule_version,
        engine_b.rules().rule_version,
        "different rulesets should produce different versions"
    );
}

// -------------------------------------------------------------------------
// evaluate_with_rules tests
// -------------------------------------------------------------------------

#[test]
fn test_evaluate_with_rules_uses_external_rules() {
    // Engine has deny-all defaults; external rules allow the request.
    let tmp = TempDir::new().unwrap();
    let engine_rules = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(engine_rules, tmp.path().to_path_buf());

    // Normal evaluate → denied (no matching rule, defaults deny)
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/data/file.txt".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "expected Denied from engine's own rules"
    );

    // External rules allow the same request
    let mut external_rules = RuleSetBuilder::new()
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
        .build()
        .unwrap();
    external_rules.compute_version();

    let resp = engine.evaluate_with_rules(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/data/file.txt".to_string(),
            op: "read".to_string(),
        }),
        None,
        &external_rules,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "expected Allowed from external rules"
    );
}

#[test]
fn test_evaluate_with_rules_different_rules_different_result() {
    // Two rule sets with opposite effects produce different evaluation results
    // for the same request.
    let tmp = TempDir::new().unwrap();
    let engine_rules = RuleSetBuilder::new().build().unwrap();
    let engine = PermissionEngine::new(engine_rules, tmp.path().to_path_buf());

    let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: "test-agent".to_string(),
        path: "/data/file.txt".to_string(),
        op: "read".to_string(),
    });

    // Rule set A: allow read
    let mut rules_a = RuleSetBuilder::new()
        .default_file(Effect::Deny)
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
        .build()
        .unwrap();
    rules_a.compute_version();

    // Rule set B: deny read
    let mut rules_b = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name("deny-read")
                .subject_agent("test-agent")
                .deny()
                .action(
                    ActionBuilder::file("read", vec!["/data/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    rules_b.compute_version();

    let resp_a = engine.evaluate_with_rules(request.clone(), None, &rules_a);
    let resp_b = engine.evaluate_with_rules(request, None, &rules_b);

    assert!(matches!(resp_a, PermissionResponse::Allowed { .. }));
    assert!(matches!(resp_b, PermissionResponse::Denied { .. }));
}

#[test]
fn test_evaluate_with_rules_empty_rules_uses_defaults() {
    // External rules with empty rule list → defaults apply
    let tmp = TempDir::new().unwrap();
    let engine_rules = RuleSetBuilder::new().build().unwrap();
    let engine = PermissionEngine::new(engine_rules, tmp.path().to_path_buf());

    let mut external_rules = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .build()
        .unwrap();
    external_rules.compute_version();

    let resp = engine.evaluate_with_rules(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/any/path".to_string(),
            op: "read".to_string(),
        }),
        None,
        &external_rules,
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
}
