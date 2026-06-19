//! Unit tests for CLI command handlers.
//!
//! Covers config validate, config list, rule check, and rule list.
//! All tests use tempfile::TempDir to avoid hardcoded paths.

use super::*;
use closeclaw::cli::args::{ConfigAction, RuleAction};
use closeclaw::permission::{Rule, RuleSet};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// config validate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_config_validate_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("good.json");
    fs::write(&file, r#"{"version":"1.0","name":"test"}"#).unwrap();

    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(result.is_ok(), "valid JSON should succeed: {:?}", result);
}

#[tokio::test]
async fn test_config_validate_valid_no_version() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("plain.json");
    fs::write(&file, r#"{"key":"value"}"#).unwrap();

    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
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

    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
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
    let result = handle_config(ConfigAction::Validate {
        file: "/nonexistent/path/config.json".to_string(),
    })
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

    let result = handle_config_with(ConfigAction::List, config_dir).await;
    assert!(result.is_ok(), "config list should succeed: {:?}", result);
}

#[tokio::test]
async fn test_config_list_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let config_dir = config_dir_for(tmp.path());

    fs::create_dir_all(&config_dir).unwrap();

    let result = handle_config_with(ConfigAction::List, config_dir).await;
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

    let result = handle_config_with(ConfigAction::List, config_dir).await;
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
    let result = handle_rule(RuleAction::Check {
        rule: json.to_string(),
    })
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
    let result = handle_rule(RuleAction::Check {
        rule: json.to_string(),
    })
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
    let result = handle_rule(RuleAction::Check {
        rule: json.to_string(),
    })
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
    let result = handle_rule(RuleAction::Check {
        rule: json.to_string(),
    })
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

    let result = handle_rule(RuleAction::Check {
        rule: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(
        result.is_ok(),
        "valid rule from file should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_rule_check_invalid_json() {
    let result = handle_rule(RuleAction::Check {
        rule: "{bad json".to_string(),
    })
    .await;
    assert!(result.is_err(), "invalid JSON should fail");
}

#[tokio::test]
async fn test_rule_check_file_not_found() {
    let result = handle_rule(RuleAction::Check {
        rule: "/nonexistent/rule.json".to_string(),
    })
    .await;
    assert!(result.is_err(), "missing file should fail");
}

// ---------------------------------------------------------------------------
// rule list
// ---------------------------------------------------------------------------

fn make_permissions(rules: Vec<Rule>) -> RuleSet {
    RuleSet {
        rules,
        defaults: closeclaw::permission::Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    }
}

fn make_rule(name: &str, agent: &str) -> Rule {
    Rule {
        name: name.to_string(),
        subject: Rule::parse_subject(agent),
        effect: closeclaw::permission::Effect::Allow,
        actions: vec![closeclaw::permission::Action::All],
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

    let result = handle_rule_with(RuleAction::List, config_dir).await;
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

    let result = handle_rule_with(RuleAction::List, config_dir).await;
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

    let result = handle_rule_with(RuleAction::List, config_dir).await;
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
    fs::write(config_sub.join("agents.json"), r#"{"agents": []}"#).unwrap();
    (tmp, config_dir)
}

/// Start an AdminServer in the background, return (config_dir, join_handle).
async fn start_mock_server(config_dir: PathBuf) -> (PathBuf, tokio::task::JoinHandle<()>) {
    let sock_path = config_dir.join("admin.sock");
    let config_manager =
        Arc::new(closeclaw::config::ConfigManager::new(config_dir.clone()).unwrap());
    let context = closeclaw::admin::server::AdminContext {
        agent_registry: Arc::new(closeclaw::agent::registry::AgentRegistry::new()),
        skill_registry: Arc::new(std::sync::RwLock::new(Some(
            closeclaw::skills::DiskSkillRegistry::default(),
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
        if tokio::net::UnixStream::connect(&sock_path).await.is_ok() {
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
    let result = handle_agent_with(AgentAction::List, config_dir).await;
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
    )
    .await;
    assert!(
        create_result.is_ok(),
        "agent create should succeed: {:?}",
        create_result
    );
    // Now list agents
    let result = handle_agent_with(AgentAction::List, config_dir).await;
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
    )
    .await
    .unwrap();
    // Get info
    let result = handle_agent_with(
        AgentAction::Info {
            name: "info-agent".into(),
        },
        config_dir,
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
    let result = handle_skill_with(SkillAction::List, config_dir).await;
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
    )
    .await;
    assert!(
        result.is_err(),
        "skill install for missing skill should fail"
    );
    handle.abort();
}
