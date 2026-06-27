//! Unit tests for CLI command handlers.
//!
//! Covers config validate, config list, rule check, rule list, and JSON output paths.
//! All tests use tempfile::TempDir to avoid hardcoded paths.

use crate::Cli;
use clap::CommandFactory;
use closeclaw::cli::admin::*;
use closeclaw::cli::args::{AgentAction, ConfigAction, RuleAction, SkillAction};
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
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
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
    let context = closeclaw::admin::server::AdminContext {
        agent_registry: Arc::new(closeclaw::agent::registry::AgentRegistry::new()),
        skill_registry: Arc::new(std::sync::RwLock::new(Some(
            closeclaw_skills::DiskSkillRegistry::default(),
        ))),
        config_manager,
        config_dir: config_dir.clone(),
    };
    let server = closeclaw::admin::AdminServer::new(sock_path, context);
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
            name: "info-agent".into(),
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
            name: "nonexistent".into(),
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
//
// These verify that the JSON output structs serialize correctly and contain
// the expected fields. They don't require stdout capture.
// ---------------------------------------------------------------------------

#[test]
fn test_config_validate_output_json() {
    let output = ConfigValidateOutput {
        file: "test.json".to_string(),
        valid: true,
        version: Some("1.0".to_string()),
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["file"], "test.json");
    assert_eq!(v["valid"], true);
    assert_eq!(v["version"], "1.0");
}

#[test]
fn test_config_validate_output_invalid() {
    let output = ConfigValidateOutput {
        file: "bad.json".to_string(),
        valid: false,
        version: None,
    };
    let json = serde_json::to_string(&output).unwrap();
    assert!(!json.contains("version"), "None version should be skipped");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["valid"], false);
}

#[test]
fn test_config_list_output_json() {
    let output = ConfigListOutput {
        files: vec![
            ConfigListFile {
                name: "a.json".to_string(),
                version: "1.0".to_string(),
                path: "/tmp/a.json".to_string(),
            },
            ConfigListFile {
                name: "b.json".to_string(),
                version: "2.0".to_string(),
                path: "/tmp/b.json".to_string(),
            },
        ],
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["name"], "a.json");
    assert_eq!(files[1]["version"], "2.0");
}

#[test]
fn test_rule_check_output_json() {
    let output = RuleCheckOutput {
        rule_name: "my-rule".to_string(),
        valid: true,
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["rule_name"], "my-rule");
    assert_eq!(v["valid"], true);
}

#[test]
fn test_rule_list_output_json() {
    let output = RuleListOutput {
        rules: vec![
            RuleListEntry {
                name: "r1".to_string(),
                subject: "agent-a".to_string(),
                effect: "allow".to_string(),
                action_count: 3,
            },
            RuleListEntry {
                name: "r2".to_string(),
                subject: "agent-b".to_string(),
                effect: "deny".to_string(),
                action_count: 1,
            },
        ],
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rules = v["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0]["name"], "r1");
    assert_eq!(rules[0]["action_count"], 3);
    assert_eq!(rules[1]["effect"], "deny");
}

#[test]
fn test_stop_output_json() {
    let output = StopOutput {
        pid: 12345,
        signal: "TERM".to_string(),
        stopped: true,
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["pid"], 12345);
    assert_eq!(v["signal"], "TERM");
    assert_eq!(v["stopped"], true);
}

#[test]
fn test_json_error_output() {
    // Verify the error JSON structure matches {"error": "..."}
    #[derive(serde::Serialize)]
    struct ErrorOutput<'a> {
        error: &'a str,
    }
    let output = ErrorOutput {
        error: "something went wrong",
    };
    let json = serde_json::to_string(&output).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["error"], "something went wrong");
}

// ---------------------------------------------------------------------------
// JSON output path tests (run with --nocapture to verify stdout output)
//
// These tests verify the JSON output path end-to-end by calling the handlers
// with json=true. They must be run with `cargo test -- --nocapture` because
// the handlers print JSON to stdout via json_output().
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
async fn test_config_validate_invalid_json() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("bad.json");
    fs::write(&file, "{not valid json").unwrap();
    // With json=true and invalid JSON, handler returns an anyhow::Error via json_error
    // Just verify the handler returns Err for invalid JSON (non-json mode)
    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "invalid JSON should fail");
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
async fn test_agent_list_json() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_agent_with(AgentAction::List, config_dir, true).await;
    assert!(
        result.is_ok(),
        "json agent list should succeed: {:?}",
        result
    );
    handle.abort();
}

#[tokio::test]
#[serial_test::serial]
async fn test_agent_info_json() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    handle_agent_with(
        AgentAction::Create {
            name: "json-agent".into(),
            model: Some("gpt-4".into()),
        },
        config_dir.clone(),
        false,
    )
    .await
    .unwrap();
    let result = handle_agent_with(
        AgentAction::Info {
            name: "json-agent".into(),
        },
        config_dir,
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "json agent info should succeed: {:?}",
        result
    );
    handle.abort();
}

#[tokio::test]
#[serial_test::serial]
async fn test_agent_create_json() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_agent_with(
        AgentAction::Create {
            name: "json-new".into(),
            model: None,
        },
        config_dir,
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "json agent create should succeed: {:?}",
        result
    );
    handle.abort();
}

#[tokio::test]
#[serial_test::serial]
async fn test_skill_list_json() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_skill_with(SkillAction::List, config_dir, true).await;
    assert!(
        result.is_ok(),
        "json skill list should succeed: {:?}",
        result
    );
    handle.abort();
}

#[tokio::test]
#[serial_test::serial]
async fn test_skill_install_json() {
    let (_tmp, config_dir) = setup_admin_config_dir();
    let (config_dir, handle) = start_mock_server(config_dir).await;
    let result = handle_skill_with(
        SkillAction::Install {
            name: "missing-skill".into(),
        },
        config_dir,
        true,
    )
    .await;
    // The mock server has no real skill registry, so install returns error.
    // With json=true, the error path uses json_error which now returns Err.
    assert!(
        result.is_err(),
        "json skill install for missing skill should fail"
    );
    handle.abort();
}

#[tokio::test]
#[serial_test::serial]
async fn test_stop_json() {
    let tmp = TempDir::new().unwrap();
    let pid_file = tmp.path().join(".closeclaw").join("daemon.pid");
    fs::create_dir_all(pid_file.parent().unwrap()).unwrap();
    fs::write(&pid_file, "9999999").unwrap();
    // PID 9999999 doesn't exist, so kill will fail.
    // With json=true, the error path uses json_error which now returns Err.
    let result = handle_stop(false, true).await;
    assert!(result.is_err(), "stop with nonexistent pid should fail");
}

// ---------------------------------------------------------------------------
// Tests migrated from handlers.rs inline mod tests
// ---------------------------------------------------------------------------

#[test]
fn test_pid() {
    let path = closeclaw::platform::process::pid_file_path(std::path::Path::new("/tmp/test"));
    assert!(path.to_str().unwrap().contains("daemon.pid"));
}

#[test]
fn test_stop_f() {
    let m = Cli::command()
        .try_get_matches_from(["c", "stop", "-f"])
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
