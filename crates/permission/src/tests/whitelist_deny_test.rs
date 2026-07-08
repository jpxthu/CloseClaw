//! Tests for whitelist deny extension (Step 1.5).
//!
//! Covers:
//! - `build_deny_rule` correctly generates Deny effect rules
//! - `append_deny_rule` write + read round-trip
//! - `append_rule` generic function: allow + deny mixed writes

use crate::engine::engine_types::{
    Action, Caller, CommandArgs, Effect, PermissionRequestBody, Rule, RuleSet,
};
use crate::whitelist::{append_deny_rule, append_rule, append_whitelist_rule, build_deny_rule};

// ── helpers ──────────────────────────────────────────────────────────────────

fn owner_caller(agent: &str) -> Caller {
    Caller {
        user_id: "owner".into(),
        agent: agent.into(),
        creator_id: String::new(),
    }
}

fn make_deny_rule(name: &str) -> Rule {
    Rule {
        name: name.into(),
        subject: crate::engine::engine_types::Subject::AgentOnly {
            agent: "a1".into(),
            match_type: Default::default(),
        },
        effect: Effect::Deny,
        actions: vec![Action::File {
            operation: "write".into(),
            paths: vec!["/etc/**".into()],
        }],
        template: None,
        priority: 0,
    }
}

fn make_allow_rule(name: &str) -> Rule {
    Rule {
        name: name.into(),
        subject: crate::engine::engine_types::Subject::AgentOnly {
            agent: "a1".into(),
            match_type: Default::default(),
        },
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: "read".into(),
            paths: vec!["/tmp/data/**".into()],
        }],
        template: None,
        priority: 0,
    }
}

fn read_ruleset(path: &std::path::Path) -> RuleSet {
    let data = std::fs::read_to_string(path).expect("file should exist");
    serde_json::from_str(&data).expect("valid RuleSet JSON")
}

// ── build_deny_rule ─────────────────────────────────────────────────────────

#[test]
fn test_build_deny_rule_file_op() {
    let caller = owner_caller("eda");
    let body = PermissionRequestBody::FileOp {
        agent: "eda".into(),
        path: "/etc/shadow".into(),
        op: "write".into(),
    };
    let rule = build_deny_rule(&caller, &body, "deny-001").unwrap();
    assert_eq!(rule.name, "deny-001");
    assert_eq!(rule.effect, Effect::Deny);
    assert!(rule.subject.is_agent_only());
    assert_eq!(rule.actions.len(), 1);
    match &rule.actions[0] {
        Action::File { operation, paths } => {
            assert_eq!(operation, "write");
            assert_eq!(paths, &["/etc/shadow"]);
        }
        other => panic!("expected Action::File, got {other:?}"),
    }
}

#[test]
fn test_build_deny_rule_command_exec() {
    let caller = owner_caller("eda");
    let body = PermissionRequestBody::CommandExec {
        agent: "eda".into(),
        cmd: "rm".into(),
        args: vec!["-rf".into(), "/".into()],
    };
    let rule = build_deny_rule(&caller, &body, "deny-cmd-001").unwrap();
    assert_eq!(rule.effect, Effect::Deny);
    match &rule.actions[0] {
        Action::Command {
            command,
            args: CommandArgs::Allowed { allowed },
        } => {
            assert_eq!(command, "rm");
            assert_eq!(allowed, &["-rf", "/"]);
        }
        other => panic!("expected Action::Command, got {other:?}"),
    }
}

#[test]
fn test_build_deny_rule_network() {
    let caller = owner_caller("eda");
    let body = PermissionRequestBody::NetOp {
        agent: "eda".into(),
        host: "evil.com".into(),
        port: 80,
    };
    let rule = build_deny_rule(&caller, &body, "deny-net-001").unwrap();
    assert_eq!(rule.effect, Effect::Deny);
    match &rule.actions[0] {
        Action::Network { hosts, ports } => {
            assert_eq!(hosts, &["evil.com"]);
            assert_eq!(ports, &[80]);
        }
        other => panic!("expected Action::Network, got {other:?}"),
    }
}

#[test]
fn test_build_deny_rule_config_write_returns_none() {
    let caller = owner_caller("eda");
    let body = PermissionRequestBody::ConfigWrite {
        agent: "eda".into(),
        config_file: "permissions.json".into(),
    };
    assert!(build_deny_rule(&caller, &body, "r1").is_none());
}

#[test]
fn test_build_deny_rule_slash_command_returns_none() {
    let caller = owner_caller("eda");
    let body = PermissionRequestBody::SlashCommand {
        agent: "eda".into(),
        command: "help".into(),
    };
    assert!(build_deny_rule(&caller, &body, "r2").is_none());
}

#[test]
fn test_build_deny_rule_name_preserved() {
    let caller = owner_caller("eda");
    let body = PermissionRequestBody::FileOp {
        agent: "eda".into(),
        path: "/tmp/f.txt".into(),
        op: "read".into(),
    };
    let rule = build_deny_rule(&caller, &body, "custom-name").unwrap();
    assert_eq!(rule.name, "custom-name");
}

// ── append_deny_rule ────────────────────────────────────────────────────────

#[test]
fn test_append_deny_rule_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let rule = make_deny_rule("deny-new-1");

    append_deny_rule(dir.path(), "agent-x", rule).unwrap();

    let path = dir
        .path()
        .join("agents")
        .join("agent-x")
        .join("permissions.json");
    assert!(path.exists());

    let rs = read_ruleset(&path);
    assert_eq!(rs.rules.len(), 1);
    assert_eq!(rs.rules[0].name, "deny-new-1");
    assert_eq!(rs.rules[0].effect, Effect::Deny);
}

#[test]
fn test_append_deny_rule_preserves_existing() {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agents").join("a1");
    std::fs::create_dir_all(&agent_dir).unwrap();

    let existing = RuleSet {
        rules: vec![make_allow_rule("old-allow")],
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&existing).unwrap();
    std::fs::write(agent_dir.join("permissions.json"), json).unwrap();

    append_deny_rule(dir.path(), "a1", make_deny_rule("new-deny")).unwrap();

    let rs = read_ruleset(&agent_dir.join("permissions.json"));
    assert_eq!(rs.rules.len(), 2);
    assert_eq!(rs.rules[0].name, "old-allow");
    assert_eq!(rs.rules[0].effect, Effect::Allow);
    assert_eq!(rs.rules[1].name, "new-deny");
    assert_eq!(rs.rules[1].effect, Effect::Deny);
}

#[test]
fn test_append_deny_rule_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    append_deny_rule(dir.path(), "brand-new-agent", make_deny_rule("first")).unwrap();

    let path = dir
        .path()
        .join("agents")
        .join("brand-new-agent")
        .join("permissions.json");
    assert!(path.exists());
    let rs = read_ruleset(&path);
    assert_eq!(rs.rules.len(), 1);
    assert_eq!(rs.rules[0].effect, Effect::Deny);
}

#[test]
fn test_append_deny_rule_corrupt_file_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agents").join("a1");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("permissions.json"), "NOT VALID JSON {{{").unwrap();

    append_deny_rule(dir.path(), "a1", make_deny_rule("after-corrupt")).unwrap();

    let rs = read_ruleset(&agent_dir.join("permissions.json"));
    assert_eq!(rs.rules.len(), 1);
    assert_eq!(rs.rules[0].name, "after-corrupt");
}

// ── append_rule mixed allow + deny ──────────────────────────────────────────

#[test]
fn test_append_rule_mixed_allow_and_deny() {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agents").join("a1");
    std::fs::create_dir_all(&agent_dir).unwrap();

    // Start with empty ruleset
    append_rule(dir.path(), "a1", make_allow_rule("allow-1")).unwrap();
    append_rule(dir.path(), "a1", make_deny_rule("deny-1")).unwrap();
    append_rule(dir.path(), "a1", make_allow_rule("allow-2")).unwrap();

    let rs = read_ruleset(&agent_dir.join("permissions.json"));
    assert_eq!(rs.rules.len(), 3);

    assert_eq!(rs.rules[0].name, "allow-1");
    assert_eq!(rs.rules[0].effect, Effect::Allow);
    assert_eq!(rs.rules[1].name, "deny-1");
    assert_eq!(rs.rules[1].effect, Effect::Deny);
    assert_eq!(rs.rules[2].name, "allow-2");
    assert_eq!(rs.rules[2].effect, Effect::Allow);
}

#[test]
fn test_append_whitelist_rule_delegates_to_append_rule() {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agents").join("a1");
    std::fs::create_dir_all(&agent_dir).unwrap();

    append_whitelist_rule(dir.path(), "a1", make_allow_rule("wl-1")).unwrap();
    append_deny_rule(dir.path(), "a1", make_deny_rule("deny-1")).unwrap();

    let rs = read_ruleset(&agent_dir.join("permissions.json"));
    assert_eq!(rs.rules.len(), 2);
    assert_eq!(rs.rules[0].effect, Effect::Allow);
    assert_eq!(rs.rules[1].effect, Effect::Deny);
}

#[test]
fn test_append_rule_round_trip_json() {
    let dir = tempfile::tempdir().unwrap();
    append_rule(dir.path(), "a1", make_deny_rule("json-check")).unwrap();

    let path = dir
        .path()
        .join("agents")
        .join("a1")
        .join("permissions.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains('\n'), "should be pretty-printed");

    let parsed: RuleSet = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed.rules[0].name, "json-check");
    assert_eq!(parsed.rules[0].effect, Effect::Deny);
}
