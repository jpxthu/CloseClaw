use tempfile::TempDir;

use super::PermissionRequestBody;

#[test]
fn test_dimension_name_file_op_read() {
    let tmp = TempDir::new().unwrap();
    let body = PermissionRequestBody::FileOp {
        agent: "a".to_string(),
        path: tmp.path().to_string_lossy().into_owned(),
        op: "read".to_string(),
    };
    assert_eq!(body.dimension_name(), Some("file_read"));
}

#[test]
fn test_dimension_name_file_op_write() {
    let tmp = TempDir::new().unwrap();
    let body = PermissionRequestBody::FileOp {
        agent: "a".to_string(),
        path: tmp.path().to_string_lossy().into_owned(),
        op: "write".to_string(),
    };
    assert_eq!(body.dimension_name(), Some("file_write"));
}

#[test]
fn test_dimension_name_file_op_unknown() {
    let tmp = TempDir::new().unwrap();
    let body = PermissionRequestBody::FileOp {
        agent: "a".to_string(),
        path: tmp.path().to_string_lossy().into_owned(),
        op: "delete".to_string(),
    };
    assert_eq!(body.dimension_name(), None);
}

#[test]
fn test_dimension_name_command_exec() {
    let body = PermissionRequestBody::CommandExec {
        agent: "a".to_string(),
        cmd: "ls".to_string(),
        args: vec![],
    };
    assert_eq!(body.dimension_name(), Some("command"));
}

#[test]
fn test_dimension_name_net_op() {
    let body = PermissionRequestBody::NetOp {
        agent: "a".to_string(),
        host: "example.com".to_string(),
        port: 443,
    };
    assert_eq!(body.dimension_name(), Some("network"));
}

#[test]
fn test_dimension_name_inter_agent_msg() {
    let body = PermissionRequestBody::InterAgentMsg {
        from: "a".to_string(),
        to: "b".to_string(),
    };
    assert_eq!(body.dimension_name(), Some("spawn"));
}

#[test]
fn test_dimension_name_tool_call() {
    let body = PermissionRequestBody::ToolCall {
        agent: "a".to_string(),
        skill: "web_search".to_string(),
        method: "run".to_string(),
    };
    assert_eq!(body.dimension_name(), Some("tool_call"));
}

#[test]
fn test_dimension_name_config_write() {
    let body = PermissionRequestBody::ConfigWrite {
        agent: "a".to_string(),
        config_file: "models.json".to_string(),
    };
    assert_eq!(body.dimension_name(), Some("config_write"));
}

#[test]
fn test_defaults_message_is_allow() {
    let defaults = super::Defaults::default();
    assert_eq!(defaults.message, super::Effect::Allow);
}

#[test]
fn test_defaults_json_missing_message() {
    // Old config without `message` field should deserialize with default Allow
    let json = r#"{"file":"deny","command":"deny","network":"deny","inter_agent":"deny","config":"deny","tool_call":"deny"}"#;
    let defaults: super::Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(defaults.message, super::Effect::Allow);
    assert_eq!(defaults.file_read, super::Effect::Deny);
    assert_eq!(defaults.file_write, super::Effect::Deny);
    assert_eq!(defaults.tool_call, super::Effect::Deny);
}

#[test]
fn test_defaults_json_with_message_allow() {
    let json = r#"{"message":"allow"}"#;
    let defaults: super::Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(defaults.message, super::Effect::Allow);
}

#[test]
fn test_defaults_json_with_message_deny() {
    let json = r#"{"message":"deny"}"#;
    let defaults: super::Defaults = serde_json::from_str(json).unwrap();
    assert_eq!(defaults.message, super::Effect::Deny);
}

#[test]
fn test_dimension_name_slash_command() {
    let body = PermissionRequestBody::SlashCommand {
        agent: "a".to_string(),
        command: "/status".to_string(),
    };
    assert_eq!(body.dimension_name(), None);
}

// ------------------------------------------------------------------
// RuleSet::compute_version tests
// ------------------------------------------------------------------

use super::{Effect, Rule, RuleSet, Subject};

fn make_rule(name: &str, agent: &str, effect: Effect) -> Rule {
    Rule {
        name: name.to_string(),
        subject: Subject::AgentOnly {
            agent: agent.to_string(),
            match_type: Default::default(),
        },
        effect,
        actions: vec![super::Action::All],
        template: None,
        priority: 0,
    }
}

#[test]
fn test_same_rules_produce_same_version() {
    let mut a = RuleSet {
        rules: vec![make_rule("r1", "agent1", Effect::Allow)],
        ..Default::default()
    };
    let mut b = RuleSet {
        rules: vec![make_rule("r1", "agent1", Effect::Allow)],
        ..Default::default()
    };
    a.compute_version();
    b.compute_version();
    assert_eq!(a.rule_version, b.rule_version);
}

#[test]
fn test_different_rules_produce_different_version() {
    let mut a = RuleSet {
        rules: vec![make_rule("r1", "agent1", Effect::Allow)],
        ..Default::default()
    };
    let mut b = RuleSet {
        rules: vec![make_rule("r1", "agent1", Effect::Deny)],
        ..Default::default()
    };
    a.compute_version();
    b.compute_version();
    assert_ne!(a.rule_version, b.rule_version);
}

#[test]
fn test_empty_ruleset_produces_valid_hash() {
    let mut rs = RuleSet::default();
    assert!(rs.rule_version.is_empty());
    rs.compute_version();
    assert!(!rs.rule_version.is_empty());
    assert_eq!(rs.rule_version.len(), 64); // SHA-256 hex = 64 chars
}

#[test]
fn test_rule_version_skipped_in_serde() {
    let mut rs = RuleSet {
        rules: vec![make_rule("r1", "agent1", Effect::Allow)],
        ..Default::default()
    };
    rs.compute_version();
    let json = serde_json::to_string(&rs).unwrap();
    assert!(!json.contains(&rs.rule_version));
    assert!(!json.contains("rule_version")); // #[serde(skip)] omits the field entirely
}

#[test]
fn test_rule_version_default_is_empty() {
    let rs = RuleSet::default();
    assert_eq!(rs.rule_version, "");
}
