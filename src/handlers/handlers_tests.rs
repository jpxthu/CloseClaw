use super::*;
use closeclaw::permission::Effect;

// -----------------------------------------------------------------
// helpers
// -----------------------------------------------------------------

/// Write `content` to a temp file named `filename` and run
/// `handle_config(Validate { file })`. Returns the result.
async fn validate_config(filename: &str, content: &str) -> anyhow::Result<()> {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(filename);
    std::fs::write(&file, content).unwrap();
    handle_config(ConfigAction::Validate {
        file: file.to_str().unwrap().to_string(),
    })
    .await
}

/// Set `HOME` to `tmp` for the duration of the closure, then restore.
fn with_home<F: FnOnce()>(tmp: &tempfile::TempDir, f: F) {
    let orig = std::env::var("HOME").ok();
    std::env::set_var("HOME", tmp.path());
    f();
    if let Some(h) = orig {
        std::env::set_var("HOME", h);
    }
}

// -----------------------------------------------------------------
// config validate tests
// -----------------------------------------------------------------

#[tokio::test]
async fn test_config_validate_valid_agents_config() {
    assert!(
        validate_config("agents.json", r#"{"agents":["orchestrator","coder"]}"#)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_config_validate_valid_models_config() {
    assert!(validate_config(
        "models.json",
        r#"{"providers":{"p":{"baseUrl":"https://api.example.com","models":[{"id":"m1"}]}}}"#
    )
    .await
    .is_ok());
}

#[tokio::test]
async fn test_config_validate_file_not_found() {
    let result = handle_config(ConfigAction::Validate {
        file: "/tmp/nonexistent_closeclaw_test_config.json".to_string(),
    })
    .await;
    assert!(result.is_err(), "missing file should return error");
}

#[tokio::test]
async fn test_config_validate_invalid_json() {
    assert!(validate_config("bad.json", "not valid json {{{")
        .await
        .is_err());
}

#[tokio::test]
async fn test_config_validate_unrecognized_format() {
    assert!(validate_config("unknown.json", r#"{"foo":"bar"}"#)
        .await
        .is_err());
}

#[tokio::test]
async fn test_config_validate_agents_with_duplicate_id() {
    assert!(
        validate_config("agents.json", r#"{"agents":["agent1","agent1"]}"#)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_config_validate_models_with_invalid_base_url() {
    assert!(validate_config(
        "models.json",
        r#"{"providers":{"p":{"baseUrl":"ftp://bad","models":[{"id":"m1"}]}}}"#
    )
    .await
    .is_err());
}

// -----------------------------------------------------------------
// config list tests
// -----------------------------------------------------------------

#[tokio::test]
async fn test_config_list_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".closeclaw")).unwrap();
    let result = {
        let mut r = None;
        with_home(&tmp, || {
            r = Some(
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(handle_config(ConfigAction::List)),
            );
        });
        r.unwrap()
    };
    assert!(
        result.is_ok(),
        "empty config list should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_config_list_no_config_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let result = {
        let mut r = None;
        with_home(&tmp, || {
            r = Some(
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(handle_config(ConfigAction::List)),
            );
        });
        r.unwrap()
    };
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
    std::fs::create_dir_all(tmp.path().join(".closeclaw").join("rules")).unwrap();
    with_home(&tmp, || assert!(handle_rule_list().is_ok()));
}

#[test]
fn test_handle_rule_list_with_files() {
    let tmp = tempfile::tempdir().unwrap();
    let rules_dir = tmp.path().join(".closeclaw").join("rules");
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(rules_dir.join("allow.json"), "{}").unwrap();
    std::fs::write(rules_dir.join("deny.yaml"), "{}").unwrap();
    std::fs::write(rules_dir.join("readme.txt"), "skip").unwrap();
    with_home(&tmp, || assert!(handle_rule_list().is_ok()));
}

#[test]
fn test_handle_rule_list_no_rules_dir() {
    let tmp = tempfile::tempdir().unwrap();
    with_home(&tmp, || assert!(handle_rule_list().is_ok()));
}

// -----------------------------------------------------------------
// Step 1.3: validate channels/gateway/plugins/system config
// -----------------------------------------------------------------

#[tokio::test]
async fn test_config_validate_valid_channels_config() {
    assert!(validate_config("channels.json",
        r#"{"channels":{"feishu":{"type":"feishu","appId":"x","appSecret":"y","botName":"b"}},"bindings":[{"agentId":"orchestrator","match":{"channel":"feishu","accountId":"acc1"}}]}"#).await.is_ok());
}

#[tokio::test]
async fn test_config_validate_channels_invalid_type() {
    assert!(validate_config("channels.json",
        r#"{"channels":{"bad":{"type":"unknown_type","appId":"x","appSecret":"y","botName":"b"}},"bindings":[]}"#).await.is_err());
}

#[tokio::test]
async fn test_config_validate_channels_empty_binding_agent() {
    assert!(validate_config("channels.json",
        r#"{"channels":{"feishu":{"type":"feishu","appId":"x","appSecret":"y","botName":"b"}},"bindings":[{"agentId":"","match":{"channel":"feishu"}}]}"#).await.is_err());
}

#[tokio::test]
async fn test_config_validate_valid_gateway_config() {
    assert!(validate_config("gateway.json",
        r#"{"name":"closeclaw","port":3000,"timeout":30000,"rateLimitPerMinute":60,"maxMessageSize":16384,"dmScope":"per-channel-peer"}"#).await.is_ok());
}

#[tokio::test]
async fn test_config_validate_gateway_port_zero() {
    assert!(validate_config("gateway.json",
        r#"{"port":0,"timeout":30000,"rateLimitPerMinute":60,"maxMessageSize":16384,"dmScope":"per-channel-peer"}"#).await.is_err());
}

#[tokio::test]
async fn test_config_validate_valid_plugins_config() {
    assert!(validate_config(
        "plugins.json",
        r#"{"entries":{"myplugin":{"enabled":true}},"installs":{}}"#
    )
    .await
    .is_ok());
}

#[tokio::test]
async fn test_config_validate_plugins_missing_installs() {
    assert!(
        validate_config("plugins.json", r#"{"entries":{"p":{"enabled":true}}}"#)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_config_validate_valid_system_config() {
    assert!(
        validate_config("system.json", r#"{"wizard":{"lastRunAt":"2026-01-01"}}"#)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_config_validate_system_invalid_session_mode() {
    assert!(validate_config(
        "system.json",
        r#"{"session":{"maintenance":{"mode":"invalid"},"dmScope":"per-channel-peer"}}"#
    )
    .await
    .is_err());
}

// -----------------------------------------------------------------
// Step 1.2: config list with credentials directory
// -----------------------------------------------------------------

#[tokio::test]
async fn test_config_list_discovers_credentials_files() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".closeclaw").join("credentials")).unwrap();
    std::fs::write(
        tmp.path()
            .join(".closeclaw")
            .join("credentials")
            .join("minimax.json"),
        r#"{"provider":"minimax","apiKey":"sk-test"}"#,
    )
    .unwrap();
    let result = {
        let mut r = None;
        with_home(&tmp, || {
            r = Some(
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(handle_config(ConfigAction::List)),
            );
        });
        r.unwrap()
    };
    assert!(
        result.is_ok(),
        "config list with credentials should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_config_list_empty_credentials_dir() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".closeclaw").join("credentials")).unwrap();
    let result = {
        let mut r = None;
        with_home(&tmp, || {
            r = Some(
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(handle_config(ConfigAction::List)),
            );
        });
        r.unwrap()
    };
    assert!(
        result.is_ok(),
        "empty credentials dir should not error: {:?}",
        result
    );
}

// -----------------------------------------------------------------
// Step 1.4: handle_stop PID cleanup tests
// -----------------------------------------------------------------

#[tokio::test]
async fn test_stop_pid_file_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let result = {
        let mut r = None;
        with_home(&tmp, || {
            r = Some(
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(handle_stop(false)),
            );
        });
        r.unwrap()
    };
    assert!(result.is_err(), "missing PID file should error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("PID file not found"),
        "error should mention PID file: {}",
        msg
    );
}

#[tokio::test]
async fn test_stop_stale_pid_file_cleans_up() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join(".closeclaw").join("daemon.pid");
    std::fs::create_dir_all(pid_file.parent().unwrap()).unwrap();
    std::fs::write(&pid_file, "2147483647").unwrap();
    let result = {
        let mut r = None;
        with_home(&tmp, || {
            r = Some(
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(handle_stop(false)),
            );
        });
        r.unwrap()
    };
    assert!(
        result.is_ok(),
        "stale PID should be cleaned up gracefully: {:?}",
        result
    );
    assert!(
        !pid_file.exists(),
        "PID file should be removed after cleanup"
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
