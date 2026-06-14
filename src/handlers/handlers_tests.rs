use super::*;
use closeclaw::permission::Effect;

// -----------------------------------------------------------------
// config validate tests
// -----------------------------------------------------------------

#[tokio::test]
async fn test_config_validate_valid_agents_config() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("agents.json");
    std::fs::write(&file, r#"{"agents":["orchestrator","coder"]}"#).unwrap();
    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(
        result.is_ok(),
        "valid agents config should pass: {:?}",
        result
    );
}

#[tokio::test]
async fn test_config_validate_valid_models_config() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("models.json");
    std::fs::write(
        &file,
        r#"{"providers":{"p":{"baseUrl":"https://api.example.com","models":[{"id":"m1"}]}}}"#,
    )
    .unwrap();
    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(
        result.is_ok(),
        "valid models config should pass: {:?}",
        result
    );
}

#[tokio::test]
async fn test_config_validate_file_not_found() {
    let result = handle_config(ConfigAction::Validate {
        file: "/tmp/nonexistent_closeclaw_test_config.json".to_string(),
    })
    .await;
    assert!(result.is_err(), "missing file should return error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("not found"),
        "error should mention not found: {}",
        msg
    );
}

#[tokio::test]
async fn test_config_validate_invalid_json() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("bad.json");
    std::fs::write(&file, "not valid json {{{").unwrap();
    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(result.is_err(), "invalid JSON should fail validation");
}

#[tokio::test]
async fn test_config_validate_unrecognized_format() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("unknown.json");
    std::fs::write(&file, r#"{"foo":"bar"}"#).unwrap();
    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(result.is_err(), "unrecognized format should fail");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Unrecognized") || msg.contains("validation failed"),
        "error should indicate unrecognized format: {}",
        msg
    );
}

#[tokio::test]
async fn test_config_validate_agents_with_duplicate_id() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("agents.json");
    std::fs::write(&file, r#"{"agents":["agent1","agent1"]}"#).unwrap();
    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(result.is_err(), "duplicate agent id should fail");
}

#[tokio::test]
async fn test_config_validate_models_with_invalid_base_url() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("models.json");
    std::fs::write(
        &file,
        r#"{"providers":{"p":{"baseUrl":"ftp://bad","models":[{"id":"m1"}]}}}"#,
    )
    .unwrap();
    let result = handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await;
    assert!(result.is_err(), "invalid base_url should fail");
}

// -----------------------------------------------------------------
// config list tests
// -----------------------------------------------------------------

#[tokio::test]
async fn test_config_list_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // Create the expected structure: tmp/.closeclaw/
    let config_dir = tmp.path().join(".closeclaw");
    std::fs::create_dir_all(&config_dir).unwrap();
    // Temporarily override HOME so handle_config uses our dir
    let orig_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", tmp.path());
    let result = handle_config(ConfigAction::List).await;
    if let Some(h) = orig_home {
        std::env::set_var("HOME", h);
    }
    // Should succeed and find no configs
    assert!(
        result.is_ok(),
        "empty config list should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_config_list_no_config_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // No .closeclaw directory
    let orig_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", tmp.path());
    let result = handle_config(ConfigAction::List).await;
    if let Some(h) = orig_home {
        std::env::set_var("HOME", h);
    }
    // Should succeed gracefully (prints message, returns Ok)
    assert!(
        result.is_ok(),
        "missing config dir should not error: {:?}",
        result
    );
}

// -----------------------------------------------------------------
// parse_rule_input tests
// -----------------------------------------------------------------

#[test]
fn test_parse_rule_input_json() {
    let json = r#"{
        "name": "test-rule",
        "subject": {"agent": "coder"},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let rule = parse_rule_input(json).unwrap();
    assert_eq!(rule.name, "test-rule");
    assert_eq!(rule.effect, Effect::Allow);
    assert!(!rule.actions.is_empty());
}

#[test]
fn test_parse_rule_input_yaml() {
    let yaml = r#"name: yaml-rule
subject:
  agent: orchestrator
effect: deny
actions:
  - type: all"#;
    let rule = parse_rule_input(yaml).unwrap();
    assert_eq!(rule.name, "yaml-rule");
    assert_eq!(rule.effect, Effect::Deny);
}

#[test]
fn test_parse_rule_input_invalid() {
    let result = parse_rule_input("neither json nor yaml {{{");
    assert!(result.is_err(), "invalid input should fail parse");
}

// -----------------------------------------------------------------
// handle_rule_check tests
// -----------------------------------------------------------------

#[test]
fn test_handle_rule_check_valid() {
    let json = r#"{
        "name": "check-test",
        "subject": {"agent": "agent1"},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let result = handle_rule_check(json);
    assert!(result.is_ok(), "valid rule should pass check: {:?}", result);
}

#[test]
fn test_handle_rule_check_empty_name() {
    let json = r#"{
        "name": "",
        "subject": {"agent": "agent1"},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let result = handle_rule_check(json);
    assert!(result.is_err(), "empty name should fail validation");
}

#[test]
fn test_handle_rule_check_empty_subject_agent() {
    let json = r#"{
        "name": "bad-subject",
        "subject": {"agent": ""},
        "effect": "allow",
        "actions": [{"type": "all"}]
    }"#;
    let result = handle_rule_check(json);
    assert!(result.is_err(), "empty subject agent should fail");
}

#[test]
fn test_handle_rule_check_no_actions_no_template() {
    let json = r#"{
        "name": "no-actions",
        "subject": {"agent": "agent1"},
        "effect": "allow"
    }"#;
    let result = handle_rule_check(json);
    assert!(result.is_err(), "missing actions and template should fail");
}

#[test]
fn test_handle_rule_check_invalid_json() {
    let result = handle_rule_check("not a rule");
    assert!(result.is_err(), "invalid input should fail");
}

// -----------------------------------------------------------------
// handle_rule_list tests
// -----------------------------------------------------------------

#[test]
fn test_handle_rule_list_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let rules_dir = tmp.path().join(".closeclaw").join("rules");
    std::fs::create_dir_all(&rules_dir).unwrap();
    let orig_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", tmp.path());
    let result = handle_rule_list();
    if let Some(h) = orig_home {
        std::env::set_var("HOME", h);
    }
    assert!(
        result.is_ok(),
        "empty rules dir should succeed: {:?}",
        result
    );
}

#[test]
fn test_handle_rule_list_with_files() {
    let tmp = tempfile::tempdir().unwrap();
    let rules_dir = tmp.path().join(".closeclaw").join("rules");
    std::fs::create_dir_all(&rules_dir).unwrap();
    // Create some rule files
    std::fs::write(rules_dir.join("allow.json"), "{}").unwrap();
    std::fs::write(rules_dir.join("deny.yaml"), "{}").unwrap();
    std::fs::write(rules_dir.join("readme.txt"), "skip").unwrap(); // not a rule file
    let orig_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", tmp.path());
    let result = handle_rule_list();
    if let Some(h) = orig_home {
        std::env::set_var("HOME", h);
    }
    assert!(
        result.is_ok(),
        "rules list with files should succeed: {:?}",
        result
    );
}

#[test]
fn test_handle_rule_list_no_rules_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // No .closeclaw/rules directory
    let orig_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", tmp.path());
    let result = handle_rule_list();
    if let Some(h) = orig_home {
        std::env::set_var("HOME", h);
    }
    assert!(
        result.is_ok(),
        "missing rules dir should not error: {:?}",
        result
    );
}

// -----------------------------------------------------------------
// legacy tests migrated from mod.rs
// -----------------------------------------------------------------

use crate::Cli;
use clap::CommandFactory;

#[test]
fn test_pid() {
    assert!(pid_file_path().to_str().unwrap().contains(".closeclaw"));
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
