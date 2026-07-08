//! Tests for whitelist rule builder
//!
//! Covers:
//! - `request_body_to_action` mapping for all `PermissionRequestBody` variants
//! - `caller_to_subject` for Owner and non-Owner callers
//! - `build_whitelist_rule` end-to-end rule construction
//! - `append_whitelist_rule` file I/O: new file, append, corrupt fallback

use crate::engine::engine_types::{
    Action, Caller, CommandArgs, Effect, PermissionRequestBody, Rule, RuleSet, Subject,
};
use crate::whitelist::{
    append_whitelist_rule, build_whitelist_rule, caller_to_subject, request_body_to_action,
};

// ── request_body_to_action ──────────────────────────────────────────────────

#[test]
fn test_file_op_read_maps_to_action() {
    let body = PermissionRequestBody::FileOp {
        agent: "a1".into(),
        path: "/tmp/file.txt".into(),
        op: "read".into(),
    };
    let action = request_body_to_action(&body).unwrap();
    match action {
        Action::File { operation, paths } => {
            assert_eq!(operation, "read");
            assert_eq!(paths, vec!["/tmp/file.txt"]);
        }
        other => panic!("expected Action::File, got {:?}", other),
    }
}

#[test]
fn test_file_op_write_maps_to_action() {
    let body = PermissionRequestBody::FileOp {
        agent: "a1".into(),
        path: "/tmp/out.txt".into(),
        op: "write".into(),
    };
    let action = request_body_to_action(&body).unwrap();
    match action {
        Action::File { operation, paths } => {
            assert_eq!(operation, "write");
            assert_eq!(paths, vec!["/tmp/out.txt"]);
        }
        other => panic!("expected Action::File, got {:?}", other),
    }
}

#[test]
fn test_command_exec_maps_to_action() {
    let body = PermissionRequestBody::CommandExec {
        agent: "a1".into(),
        cmd: "ls".into(),
        args: vec!["-la".into(), "/tmp".into()],
    };
    let action = request_body_to_action(&body).unwrap();
    match action {
        Action::Command {
            command,
            args: CommandArgs::Allowed { allowed },
        } => {
            assert_eq!(command, "ls");
            assert_eq!(allowed, vec!["-la", "/tmp"]);
        }
        other => panic!("expected Action::Command, got {:?}", other),
    }
}

#[test]
fn test_net_op_maps_to_action() {
    let body = PermissionRequestBody::NetOp {
        agent: "a1".into(),
        host: "example.com".into(),
        port: 443,
    };
    let action = request_body_to_action(&body).unwrap();
    match action {
        Action::Network { hosts, ports } => {
            assert_eq!(hosts, vec!["example.com"]);
            assert_eq!(ports, vec![443]);
        }
        other => panic!("expected Action::Network, got {:?}", other),
    }
}

#[test]
fn test_tool_call_maps_to_action() {
    let body = PermissionRequestBody::ToolCall {
        agent: "a1".into(),
        skill: "web_search".into(),
        method: "search".into(),
    };
    let action = request_body_to_action(&body).unwrap();
    match action {
        Action::ToolCall { skill, methods } => {
            assert_eq!(skill, "web_search");
            assert_eq!(methods, vec!["search"]);
        }
        other => panic!("expected Action::ToolCall, got {:?}", other),
    }
}

#[test]
fn test_inter_agent_msg_maps_to_action() {
    let body = PermissionRequestBody::InterAgentMsg {
        from: "agent-a".into(),
        to: "agent-b".into(),
    };
    let action = request_body_to_action(&body).unwrap();
    match action {
        Action::InterAgent { agents } => {
            assert_eq!(agents, vec!["agent-b"]);
        }
        other => panic!("expected Action::InterAgent, got {:?}", other),
    }
}

#[test]
fn test_config_write_returns_none() {
    let body = PermissionRequestBody::ConfigWrite {
        agent: "a1".into(),
        config_file: "permissions.json".into(),
    };
    assert!(request_body_to_action(&body).is_none());
}

#[test]
fn test_slash_command_returns_none() {
    let body = PermissionRequestBody::SlashCommand {
        agent: "a1".into(),
        command: "help".into(),
    };
    assert!(request_body_to_action(&body).is_none());
}

// ── caller_to_subject ───────────────────────────────────────────────────────

#[test]
fn test_owner_caller_produces_agent_only() {
    let caller = Caller {
        user_id: "owner".into(),
        agent: "a1".into(),
        creator_id: String::new(),
    };
    let subject = caller_to_subject(&caller);
    assert!(subject.is_agent_only());
    assert_eq!(subject.agent_id(), "a1");
}

#[test]
fn test_empty_user_id_produces_agent_only() {
    let caller = Caller {
        user_id: String::new(),
        agent: "a2".into(),
        creator_id: String::new(),
    };
    let subject = caller_to_subject(&caller);
    assert!(subject.is_agent_only());
    assert_eq!(subject.agent_id(), "a2");
}

#[test]
fn test_non_owner_with_user_id_produces_user_and_agent() {
    let caller = Caller {
        user_id: "ou_alice".into(),
        agent: "a3".into(),
        creator_id: "creator-1".into(),
    };
    let subject = caller_to_subject(&caller);
    assert!(subject.is_user_and_agent());
    assert_eq!(subject.user_id(), "ou_alice");
    assert_eq!(subject.agent_id(), "a3");
}

// ── build_whitelist_rule ────────────────────────────────────────────────────

#[test]
fn test_build_rule_for_file_op() {
    let caller = Caller {
        user_id: "ou_alice".into(),
        agent: "a1".into(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::FileOp {
        agent: "a1".into(),
        path: "/tmp/data.txt".into(),
        op: "read".into(),
    };
    let rule = build_whitelist_rule(&caller, &body, "whitelist-001").unwrap();
    assert_eq!(rule.name, "whitelist-001");
    assert_eq!(rule.effect, Effect::Allow);
    assert!(rule.actions.len() == 1);
    assert!(rule.subject.is_user_and_agent());
    assert!(rule.template.is_none());
    assert_eq!(rule.priority, 0);
}

#[test]
fn test_build_rule_for_config_write_returns_none() {
    let caller = Caller {
        user_id: "owner".into(),
        agent: "a1".into(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::ConfigWrite {
        agent: "a1".into(),
        config_file: "permissions.json".into(),
    };
    assert!(build_whitelist_rule(&caller, &body, "r1").is_none());
}

#[test]
fn test_build_rule_for_slash_command_returns_none() {
    let caller = Caller {
        user_id: "owner".into(),
        agent: "a1".into(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::SlashCommand {
        agent: "a1".into(),
        command: "help".into(),
    };
    assert!(build_whitelist_rule(&caller, &body, "r2").is_none());
}

#[test]
fn test_build_rule_owner_caller_agent_only_subject() {
    let caller = Caller {
        user_id: "owner".into(),
        agent: "a1".into(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::ToolCall {
        agent: "a1".into(),
        skill: "web".into(),
        method: "fetch".into(),
    };
    let rule = build_whitelist_rule(&caller, &body, "wl-owner").unwrap();
    assert!(rule.subject.is_agent_only());
    assert_eq!(rule.effect, Effect::Allow);
}

// ── append_whitelist_rule ──────────────────────────────────────────────────

fn make_rule(name: &str) -> Rule {
    Rule {
        name: name.into(),
        subject: Subject::AgentOnly {
            agent: "a1".into(),
            match_type: Default::default(),
        },
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: "read".into(),
            paths: vec!["/tmp/f.txt".into()],
        }],
        template: None,
        priority: 0,
    }
}

fn read_ruleset(path: &std::path::Path) -> RuleSet {
    let data = std::fs::read_to_string(path).expect("file should exist");
    serde_json::from_str(&data).expect("valid RuleSet JSON")
}

#[test]
fn test_append_to_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let rule = make_rule("wl-new-1");

    append_whitelist_rule(dir.path(), "agent-x", rule).unwrap();

    let path = dir
        .path()
        .join("agents")
        .join("agent-x")
        .join("permissions.json");
    assert!(path.exists());

    let rs = read_ruleset(&path);
    assert_eq!(rs.rules.len(), 1);
    assert_eq!(rs.rules[0].name, "wl-new-1");
}

#[test]
fn test_append_preserves_existing_rules() {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agents").join("a1");
    std::fs::create_dir_all(&agent_dir).unwrap();

    let existing = RuleSet {
        rules: vec![make_rule("old-rule")],
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&existing).unwrap();
    std::fs::write(agent_dir.join("permissions.json"), json).unwrap();

    append_whitelist_rule(dir.path(), "a1", make_rule("new-rule")).unwrap();

    let rs = read_ruleset(&agent_dir.join("permissions.json"));
    assert_eq!(rs.rules.len(), 2);
    assert_eq!(rs.rules[0].name, "old-rule");
    assert_eq!(rs.rules[1].name, "new-rule");
}

#[test]
fn test_append_to_corrupt_file_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agents").join("a1");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("permissions.json"), "not valid json {{{").unwrap();

    append_whitelist_rule(dir.path(), "a1", make_rule("after-corrupt")).unwrap();

    let rs = read_ruleset(&agent_dir.join("permissions.json"));
    assert_eq!(rs.rules.len(), 1);
    assert_eq!(rs.rules[0].name, "after-corrupt");
}

#[test]
fn test_append_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    // agents/brand-new-agent/ does not exist yet
    append_whitelist_rule(dir.path(), "brand-new-agent", make_rule("first")).unwrap();

    let path = dir
        .path()
        .join("agents")
        .join("brand-new-agent")
        .join("permissions.json");
    assert!(path.exists());
    let rs = read_ruleset(&path);
    assert_eq!(rs.rules.len(), 1);
}

#[test]
fn test_append_output_is_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    append_whitelist_rule(dir.path(), "a1", make_rule("json-check")).unwrap();

    let path = dir
        .path()
        .join("agents")
        .join("a1")
        .join("permissions.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    // Should be pretty-printed (contains newlines)
    assert!(raw.contains('\n'));
    // Round-trip: parse back
    let parsed: RuleSet = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed.rules[0].name, "json-check");
}
