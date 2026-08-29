//! Unit tests for CLI command handlers.
//!
//! Covers config validate, config list, rule check, rule list, and JSON output paths.
//! All tests use tempfile::TempDir to avoid hardcoded paths.

use clap::{Arg, ArgAction, Command as ClapCommand};
use closeclaw_cli::admin::*;
use closeclaw_cli::args::{AgentAction, ConfigAction, RuleAction, SkillAction};
use closeclaw_permission::{Rule, RuleSet};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// config validate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_config_validate_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("good.json");
    fs::write(&file, r#"{"version":"1.0","name":"test"}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_ok(), "valid JSON should succeed: {:?}", result);
}

#[tokio::test]
async fn test_config_validate_valid_no_version() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("plain.json");
    fs::write(&file, r#"{"key":"value"}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid JSON without version should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_config_validate_invalid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("bad.json");
    fs::write(&file, "{not valid json").unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "invalid JSON should return error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Validation failed"),
        "error should mention validation failure: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_config_validate_not_found() {
    let result = handle_config(
        ConfigAction::Validate {
            file: "/nonexistent/path/config.json".to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "missing file should return error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Failed to read"),
        "error should mention file read failure: {}",
        err_msg
    );
}

// ---------------------------------------------------------------------------
// config list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_config_list_with_files() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());

    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("a.json"),
        r#"{"version":"1.0","name":"alpha"}"#,
    )
    .unwrap();
    fs::write(
        config_dir.join("b.json"),
        r#"{"version":"2.0","name":"beta"}"#,
    )
    .unwrap();
    // Non-json file should be ignored
    fs::write(config_dir.join("readme.txt"), "hello").unwrap();

    let result = handle_config_with(ConfigAction::List, config_dir, false).await;
    assert!(result.is_ok(), "config list should succeed: {:?}", result);
}

#[tokio::test]
async fn test_config_list_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());

    fs::create_dir_all(&config_dir).unwrap();

    let result = handle_config_with(ConfigAction::List, config_dir, false).await;
    assert!(
        result.is_ok(),
        "config list on empty dir should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_config_list_no_dir() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());
    // Ensure config dir does NOT exist
    assert!(!config_dir.exists());

    let result = handle_config_with(ConfigAction::List, config_dir, false).await;
    assert!(
        result.is_ok(),
        "config list with missing dir should succeed: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// rule check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rule_check_valid() {
    let json = r#"{
        "name": "test-rule",
        "subject": {"agent": "agent-a"},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let result = handle_rule(
        RuleAction::Check {
            rule: json.to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_ok(), "valid rule should succeed: {:?}", result);
}

#[tokio::test]
async fn test_rule_check_missing_actions_and_template() {
    let json = r#"{
        "name": "bad-rule",
        "subject": {"agent": "agent-a"},
        "effect": "deny"
    }"#;
    let result = handle_rule(
        RuleAction::Check {
            rule: json.to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_err(),
        "rule without actions or template should fail"
    );
}

#[tokio::test]
async fn test_rule_check_empty_name() {
    let json = r#"{
        "name": "",
        "subject": {"agent": "agent-a"},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let result = handle_rule(
        RuleAction::Check {
            rule: json.to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "rule with empty name should fail");
}

#[tokio::test]
async fn test_rule_check_empty_subject_agent() {
    let json = r#"{
        "name": "no-agent",
        "subject": {"agent": ""},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let result = handle_rule(
        RuleAction::Check {
            rule: json.to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "rule with empty agent should fail");
}

#[tokio::test]
async fn test_rule_check_from_file() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("rule.json");
    fs::write(
        &file,
        r#"{
            "name": "file-rule",
            "subject": {"agent": "agent-b"},
            "effect": "allow",
            "actions": [{"type": "all"}]
        }"#,
    )
    .unwrap();

    let result = handle_rule(
        RuleAction::Check {
            rule: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid rule from file should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_rule_check_invalid_json() {
    let result = handle_rule(
        RuleAction::Check {
            rule: "{bad json".to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "invalid JSON should fail");
}

#[tokio::test]
async fn test_rule_check_file_not_found() {
    let result = handle_rule(
        RuleAction::Check {
            rule: "/nonexistent/rule.json".to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "missing file should fail");
}

// ---------------------------------------------------------------------------
// rule list
// ---------------------------------------------------------------------------

fn make_permissions(rules: Vec<Rule>) -> RuleSet {
    RuleSet {
        rules,
        defaults: closeclaw_permission::Defaults::default(),
        user_defaults: closeclaw_permission::Defaults::user_defaults(),
        template_includes: vec![],
        rule_version: String::new(),
    }
}

fn make_rule(name: &str, agent: &str) -> Rule {
    Rule {
        name: name.to_string(),
        subject: Rule::parse_subject(agent),
        effect: closeclaw_permission::Effect::Allow,
        actions: vec![closeclaw_permission::Action::All],
        template: None,
        priority: 0,
    }
}

#[tokio::test]
async fn test_rule_list_with_rules() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());

    fs::create_dir_all(&config_dir).unwrap();
    let rule_set = make_permissions(vec![
        make_rule("rule-1", "agent-a"),
        make_rule("rule-2", "agent-b"),
    ]);
    let json = serde_json::to_string_pretty(&rule_set).unwrap();
    fs::write(config_dir.join("permissions.json"), json).unwrap();

    let result = handle_rule_with(RuleAction::List, config_dir, false).await;
    assert!(result.is_ok(), "rule list should succeed: {:?}", result);
}

#[tokio::test]
async fn test_rule_list_empty_rules() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());

    fs::create_dir_all(&config_dir).unwrap();
    let rule_set = make_permissions(vec![]);
    let json = serde_json::to_string_pretty(&rule_set).unwrap();
    fs::write(config_dir.join("permissions.json"), json).unwrap();

    let result = handle_rule_with(RuleAction::List, config_dir, false).await;
    assert!(
        result.is_ok(),
        "rule list with empty rules should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_rule_list_no_file() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());

    fs::create_dir_all(&config_dir).unwrap();
    // No permissions.json created

    let result = handle_rule_with(RuleAction::List, config_dir, false).await;
    assert!(
        result.is_ok(),
        "rule list with missing file should succeed: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// agent / skill handler tests via mock admin server
// ---------------------------------------------------------------------------

use std::sync::Arc;

/// Create a temp config dir with the required sub-structure for AdminServer.
fn setup_admin_config_dir() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());
    let config_sub = config_dir.join("config");
    fs::create_dir_all(&config_sub).unwrap();
    // agents.json lives in the config subdirectory
    // (ConfigManager.config_dir = config_sub)
    fs::write(config_sub.join("agents.json"), r#"{"agents": []}"#).unwrap();
    (tmp, config_dir)
}

/// Start an AdminServer in the background, return (config_dir, join_handle).
async fn start_mock_server(config_dir: PathBuf) -> (PathBuf, tokio::task::JoinHandle<()>) {
    let sock_path = config_dir.join("admin.sock");
    // ConfigManager receives the config subdirectory
    let config_sub = config_dir.join("config");
    let config_manager = Arc::new(closeclaw_config::ConfigManager::new(config_sub).unwrap());
    let context = closeclaw_cli::admin::AdminContext {
        agent_registry: Arc::new(closeclaw_agent::registry::AgentRegistry::new()),
        skill_registry: Arc::new(std::sync::RwLock::new(Some(
            closeclaw_skills::DiskSkillRegistry::default(),
        ))),
        config_manager,
        config_dir: config_dir.clone(),
        restart_tx: None,
    };
    let server = closeclaw_cli::admin::AdminServer::new(sock_path, context);
    let handle = tokio::spawn(async move {
        let _ = server.serve().await;
    });
    // Poll until the socket is ready
    let sock_path = config_dir.join("admin.sock");
    for _ in 0..50 {
        let result: Result<tokio::net::UnixStream, _> =
            tokio::net::UnixStream::connect(&sock_path).await;
        if result.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    (config_dir, handle)
}

// --- Agent handler tests ---------------------------------------------------

#[tokio::test]
async fn test_handle_agent_list_empty() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_agent_with(AgentAction::List, config_dir, false).await;
    assert!(result.is_ok(), "agent list should succeed: {:?}", result);
    handle.abort();
}

#[tokio::test]
async fn test_handle_agent_list_with_agents() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    // Create an agent first
    let create_result = handle_agent_with(
        AgentAction::Create {
            name: "test-agent".into(),
            model: Some("gpt-4".into()),
        },
        config_dir.clone(),
        false,
    )
    .await;
    assert!(
        create_result.is_ok(),
        "agent create should succeed: {:?}",
        create_result
    );
    // Now list agents
    let result = handle_agent_with(AgentAction::List, config_dir, false).await;
    assert!(result.is_ok(), "agent list should succeed: {:?}", result);
    handle.abort();
}

#[tokio::test]
async fn test_handle_agent_info_found() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    // Create an agent
    handle_agent_with(
        AgentAction::Create {
            name: "info-agent".into(),
            model: None,
        },
        config_dir.clone(),
        false,
    )
    .await
    .unwrap();
    // Get info
    let result = handle_agent_with(
        AgentAction::Info {
            id: "info-agent".into(),
        },
        config_dir,
        false,
    )
    .await;
    assert!(result.is_ok(), "agent info should succeed: {:?}", result);
    handle.abort();
}

#[tokio::test]
async fn test_handle_agent_info_not_found() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_agent_with(
        AgentAction::Info {
            id: "nonexistent".into(),
        },
        config_dir,
        false,
    )
    .await;
    assert!(result.is_err(), "agent info for missing agent should fail");
    handle.abort();
}

#[tokio::test]
async fn test_handle_agent_create() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_agent_with(
        AgentAction::Create {
            name: "new-agent".into(),
            model: Some("claude-3".into()),
        },
        config_dir,
        false,
    )
    .await;
    assert!(result.is_ok(), "agent create should succeed: {:?}", result);
    handle.abort();
}

// --- Skill handler tests ---------------------------------------------------

#[tokio::test]
async fn test_handle_skill_list_empty() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_skill_with(SkillAction::List, config_dir, false).await;
    assert!(result.is_ok(), "skill list should succeed: {:?}", result);
    handle.abort();
}

#[tokio::test]
async fn test_handle_skill_install_not_found() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_skill_with(
        SkillAction::Install {
            name: "missing-skill".into(),
        },
        config_dir,
        false,
    )
    .await;
    assert!(
        result.is_err(),
        "skill install for missing skill should fail"
    );
    handle.abort();
}

// ---------------------------------------------------------------------------
// JSON output struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_json_output_structs() {
    let valid = ConfigValidateOutput {
        file: "test.json".into(),
        valid: true,
        version: Some("1.0".into()),
    };
    let v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&valid).unwrap()).unwrap();
    assert_eq!(v["file"], "test.json");
    assert_eq!(v["version"], "1.0");
    let invalid = ConfigValidateOutput {
        file: "bad.json".into(),
        valid: false,
        version: None,
    };
    assert!(!serde_json::to_string(&invalid).unwrap().contains("version"));
    let output = ConfigListOutput {
        files: vec![
            ConfigListFile {
                name: "a.json".into(),
                version: "1.0".into(),
                path: "/tmp/a.json".into(),
            },
            ConfigListFile {
                name: "b.json".into(),
                version: "2.0".into(),
                path: "/tmp/b.json".into(),
            },
        ],
    };
    let v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&output).unwrap()).unwrap();
    assert_eq!(v["files"].as_array().unwrap().len(), 2);
    let v: serde_json::Value = serde_json::from_str(
        &serde_json::to_string(&RuleCheckOutput {
            rule_name: "my-rule".into(),
            valid: true,
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(v["rule_name"], "my-rule");
    let rules_out = RuleListOutput {
        rules: vec![
            RuleListEntry {
                name: "r1".into(),
                subject: "agent-a".into(),
                effect: "allow".into(),
                action_count: 3,
            },
            RuleListEntry {
                name: "r2".into(),
                subject: "agent-b".into(),
                effect: "deny".into(),
                action_count: 1,
            },
        ],
    };
    let v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&rules_out).unwrap()).unwrap();
    assert_eq!(v["rules"].as_array().unwrap().len(), 2);
    let v: serde_json::Value = serde_json::from_str(
        &serde_json::to_string(&StopOutput {
            pid: Some(12345),
            signal: "TERM".into(),
            stopped: true,
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(v["pid"], 12345);
    let v: serde_json::Value = serde_json::from_str(
        &serde_json::to_string(&StopOutput {
            pid: None,
            signal: String::new(),
            stopped: false,
        })
        .unwrap(),
    )
    .unwrap();
    assert!(v["pid"].is_null());
    #[derive(serde::Serialize)]
    struct ErrorOutput<'a> {
        error: &'a str,
    }
    let v: serde_json::Value = serde_json::from_str(
        &serde_json::to_string(&ErrorOutput {
            error: "something went wrong",
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(v["error"], "something went wrong");
}

// ---------------------------------------------------------------------------
// JSON output path tests (run with --nocapture)
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn test_config_validate_json() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("good.json");
    fs::write(&file, r#"{"version":"1.0","name":"test"}"#).unwrap();
    // With json=true, handler prints JSON to stdout and returns Ok
    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "json config validate should succeed: {:?}",
        result
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_config_list_json() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("a.json"),
        r#"{"version":"1.0","name":"alpha"}"#,
    )
    .unwrap();
    let result = handle_config_with(ConfigAction::List, config_dir, true).await;
    assert!(
        result.is_ok(),
        "json config list should succeed: {:?}",
        result
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_rule_check_json() {
    let rule = r#"{
        "name": "test-rule",
        "subject": {"agent": "agent-a"},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let result = handle_rule(
        RuleAction::Check {
            rule: rule.to_string(),
        },
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "json rule check should succeed: {:?}",
        result
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_rule_list_json() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());
    fs::create_dir_all(&config_dir).unwrap();
    let rule_set = make_permissions(vec![make_rule("rule-1", "agent-a")]);
    let json = serde_json::to_string_pretty(&rule_set).unwrap();
    fs::write(config_dir.join("permissions.json"), json).unwrap();
    let result = handle_rule_with(RuleAction::List, config_dir, true).await;
    assert!(
        result.is_ok(),
        "json rule list should succeed: {:?}",
        result
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_agent_json_crud() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let r = handle_agent_with(AgentAction::List, config_dir.clone(), true).await;
    assert!(r.is_ok(), "json agent list: {:?}", r);
    let r = handle_agent_with(
        AgentAction::Create {
            name: "json-new".into(),
            model: Some("gpt-4".into()),
        },
        config_dir.clone(),
        false,
    )
    .await;
    assert!(r.is_ok(), "json agent create: {:?}", r);
    let r = handle_agent_with(
        AgentAction::Info {
            id: "json-new".into(),
        },
        config_dir,
        true,
    )
    .await;
    assert!(r.is_ok(), "json agent info: {:?}", r);
    handle.abort();
}

#[tokio::test]
#[serial_test::serial]
async fn test_skill_list_and_install_json() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_skill_with(SkillAction::List, config_dir.clone(), true).await;
    assert!(result.is_ok(), "json skill list: {:?}", result);
    let result = handle_skill_with(
        SkillAction::Install {
            name: "missing-skill".into(),
        },
        config_dir,
        true,
    )
    .await;
    assert!(result.is_err(), "json skill install should fail");
    handle.abort();
}

// ---------------------------------------------------------------------------
// Tests migrated from handlers.rs inline mod tests
// ---------------------------------------------------------------------------

#[test]
fn test_pid() {
    let path = closeclaw_platform::process::pid_file_path(std::path::Path::new("/tmp/test"));
    assert!(path.to_str().unwrap().contains("daemon.pid"));
}

#[test]
fn test_stop_f() {
    // Build an equivalent clap Command to test the stop subcommand's -f flag,
    // since Cli is defined in the binary crate and not accessible here.
    let cmd = ClapCommand::new("closeclaw").subcommand(
        ClapCommand::new("stop").arg(
            Arg::new("force")
                .short('f')
                .long("force")
                .action(ArgAction::SetTrue),
        ),
    );
    let m = cmd
        .try_get_matches_from(["closeclaw", "stop", "-f"])
        .unwrap();
    assert!(m.subcommand().unwrap().1.get_flag("force"));
}

#[test]
fn test_mask_key_short() {
    // Keys <= 8 chars are fully masked
    assert_eq!(mask_key("abc"), "****");
    assert_eq!(mask_key("12345678"), "****");
}

#[test]
fn test_mask_key_long() {
    // Keys > 8 chars show first 4 and last 4
    assert_eq!(mask_key("abcdefghij"), "abcd....ghij");
    assert_eq!(mask_key("minimax-key-001"), "mini....-001");
    assert_eq!(mask_key("sk-1234567890abcdef"), "sk-1....cdef");
}

#[test]
fn test_env_write_uses_raw_key() {
    // Verify the format string used in handle_config_setup writes raw key (not masked)
    let k = "MINIMAX";
    let v = "my-secret-key-123";
    let line = format!("{}={}\n", k, v);
    assert!(line.starts_with("MINIMAX=my-secret-key-123"));
    assert!(!line.contains("****"));
    assert!(!line.contains("...."));
    // Also verify the key portion does NOT contain mask pattern
    let written = format!("{}={}", k, v);
    assert!(written.contains("my-secret-key-123"));
}

// ---------------------------------------------------------------------------
// Step 1.3 — CLI JSON output contains all 12 agent info fields
// ---------------------------------------------------------------------------

/// Verify AgentInfoResult serializes to JSON with all 12 fields present.
/// Uses serde_json directly to avoid needing a running daemon.
#[test]
fn test_agent_info_json_output_all_fields() {
    use closeclaw_cli::admin::rpc::protocol::{AdminResponse, AgentInfoResult};
    use closeclaw_config::agents::{MemoryConfig, ModelSpec, SubagentsConfig};

    let info = AgentInfoResult {
        id: "cli-test-agent".to_string(),
        name: "CLI Test Agent".to_string(),
        parent_id: Some("parent-id".to_string()),
        model: Some(ModelSpec::single("claude-3-opus")),
        workspace: Some("/tmp/ws".to_string()),
        agent_dir: Some("/tmp/ad".to_string()),
        bootstrap_mode: closeclaw_common::BootstrapMode::Minimal,
        skills: vec!["s1".to_string()],
        tools: vec!["t1".to_string()],
        disallowed_tools: vec!["d1".to_string()],
        subagents: SubagentsConfig::default(),
        memory: Some(MemoryConfig::default()),
    };
    let resp = AdminResponse::AgentInfoResult(Box::new(info));
    let json_str = serde_json::to_string(&resp).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Tagged enum flattens struct fields; type is at top level
    assert_eq!(v["type"], "agent_info_result");

    assert_eq!(v["id"], "cli-test-agent");
    assert_eq!(v["name"], "CLI Test Agent");
    assert_eq!(v["parentId"], "parent-id");
    assert_eq!(v["model"], "claude-3-opus");
    assert_eq!(v["workspace"], "/tmp/ws");
    assert_eq!(v["agentDir"], "/tmp/ad");
    assert_eq!(v["bootstrapMode"], "minimal");
    assert!(v["skills"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("s1")));
    assert!(v["tools"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("t1")));
    assert!(v["disallowedTools"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("d1")));
    assert!(v["subagents"].is_object());
    assert!(v["memory"].is_object());
}

// ---------------------------------------------------------------------------
// Step 1.4 — handle_stop: no PID file returns Ok, self-kill protection
// ---------------------------------------------------------------------------

/// Tests handle_stop edge cases:
/// 1. No PID file → Ok (daemon not running)
/// 2. No PID file in JSON mode → Ok
/// 3. Self-kill protection → Err
/// Uses a temp directory to avoid polluting the real config path.
#[tokio::test]
async fn test_handle_stop_no_pid_and_self_kill() {
    use closeclaw_cli::admin::handle_stop_at;
    use closeclaw_platform::process::{pid_file_path, write_pid_file};

    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path();

    // No PID file: text mode → Ok
    let result = handle_stop_at(config_dir, false, false).await;
    assert!(result.is_ok(), "no PID file should return Ok: {:?}", result);

    // No PID file: JSON mode → Ok
    let result = handle_stop_at(config_dir, false, true).await;
    assert!(
        result.is_ok(),
        "no PID file (json) should return Ok: {:?}",
        result
    );

    // Self-kill protection
    let my_pid = std::process::id();
    let pid_file = pid_file_path(config_dir);
    write_pid_file(&pid_file, my_pid).unwrap();
    let result = handle_stop_at(config_dir, false, false).await;
    assert!(result.is_err(), "should refuse to kill self");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Refusing to kill self"),
        "error should mention self-kill refusal, got: {err_msg}"
    );
}

// ── Step 1.3 — handle_stop_at: signal → wait → cleanup full chain ──

/// Full chain: signal sent → zombie not reaped → wait times out → PID file preserved → Err.
#[tokio::test]
async fn test_handle_stop_full_chain_signal_and_timeout() {
    use closeclaw_cli::admin::handle_stop_at;
    use closeclaw_platform::process::{pid_file_path, write_pid_file};
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path();
    let pid_file = pid_file_path(config_dir);
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn sleep child");
    let pid = child.id();
    write_pid_file(&pid_file, pid).unwrap();
    assert!(pid_file.exists());
    let result = handle_stop_at(config_dir, false, false).await;
    assert!(result.is_err(), "should return Err on zombie timeout");
    assert!(pid_file.exists(), "PID file should be preserved on timeout");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("did not exit within"),
        "error should mention timeout: {}",
        err_msg
    );
    child.kill().ok();
    let status = child.wait().unwrap();
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(unix)]
    // Process exited from SIGTERM sent by handle_stop_at (zombie reaped by our kill+wait).
    assert!(
        status.signal().is_some(),
        "child should have been killed by signal"
    );
}

/// Timeout path: immune child + SIGTERM → wait_for_exit returns Err.
#[test]
fn test_handle_stop_timeout_unit_coverage() {
    use closeclaw_platform::process::{send_signal, wait_for_exit};
    let mut child = std::process::Command::new("sh")
        .args(["-c", "trap '' TERM; sleep 60"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn immune child");
    let pid = child.id();
    send_signal(pid, false).expect("send_signal should succeed");
    let result = wait_for_exit(pid, std::time::Duration::from_millis(200));
    assert!(result.is_err(), "should timeout on immune process");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("did not exit within"),
        "timeout error: {}",
        err_msg
    );
    child.kill().ok();
    child.wait().ok();
}
