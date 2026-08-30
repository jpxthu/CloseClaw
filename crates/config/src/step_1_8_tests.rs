//! Step 1.8 — Unit tests for Steps 1.1–1.7.
//!
//! Validates all behavior dimensions specified in the plan without
//! modifying any business code.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

use crate::manager::ConfigSection;
use crate::providers::accounts::{AccountsConfigData, BotAgentBinding};
use crate::providers::credentials::CredentialsProvider;
use crate::providers::ConfigProvider;
use crate::session::{JsonSessionConfigProvider, SessionConfig, SessionConfigProvider};
use crate::validators::{for_section, validate_credentials};

// =========================================================================
// Step 1.1 — SessionConfig plan_archive_days / audit_log_limit deserialization + defaults
// =========================================================================

#[test]
fn test_session_config_deserializes_plan_archive_days_and_audit_log_limit() {
    let json = r#"{
        "defaults": {},
        "agents": {},
        "sweeperIntervalSeconds": 300,
        "planArchiveDays": 14,
        "auditLogLimit": 2000
    }"#;
    let config: SessionConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.plan_archive_days, 14);
    assert_eq!(config.audit_log_limit, 2000);
}

#[test]
fn test_session_config_defaults_contain_plan_archive_days_and_audit_log_limit() {
    let config = SessionConfig::default();
    assert_eq!(config.plan_archive_days, 7);
    assert_eq!(config.audit_log_limit, 1000);
}

#[test]
fn test_session_config_missing_plan_archive_days_and_audit_log_limit_uses_defaults() {
    let json = r#"{"defaults": {}, "agents": {}, "sweeperIntervalSeconds": 300}"#;
    let config: SessionConfig = serde_json::from_str(json).unwrap();
    // Missing fields → serde default (7 for u64, 1000 for usize)
    assert_eq!(config.plan_archive_days, 7);
    assert_eq!(config.audit_log_limit, 1000);
}

#[test]
fn test_session_config_plan_archive_days_zero_is_valid() {
    let json = r#"{
        "defaults": {},
        "agents": {},
        "sweeperIntervalSeconds": 300,
        "planArchiveDays": 0
    }"#;
    let config: SessionConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.plan_archive_days, 0);
}

#[test]
fn test_session_config_audit_log_limit_zero_is_valid() {
    let json = r#"{
        "defaults": {},
        "agents": {},
        "sweeperIntervalSeconds": 300,
        "auditLogLimit": 0
    }"#;
    let config: SessionConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.audit_log_limit, 0);
}

#[test]
fn test_session_config_provider_returns_plan_archive_days() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("session.json");
    fs::write(
        &path,
        r#"{
            "defaults": {},
            "agents": {},
            "sweeperIntervalSeconds": 300,
            "planArchiveDays": 30
        }"#,
    )
    .unwrap();
    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    assert_eq!(provider.plan_archive_days(), 30);
}

#[test]
fn test_session_config_provider_returns_audit_log_limit() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("session.json");
    fs::write(
        &path,
        r#"{
            "defaults": {},
            "agents": {},
            "sweeperIntervalSeconds": 300,
            "auditLogLimit": 500
        }"#,
    )
    .unwrap();
    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    assert_eq!(provider.audit_log_limit(), 500);
}

#[test]
fn test_session_config_provider_defaults_when_file_absent() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("nonexistent.json");
    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    // File absent → defaults
    assert_eq!(provider.plan_archive_days(), 7);
    assert_eq!(provider.audit_log_limit(), 1000);
}

// =========================================================================
// Step 1.2 — ConfigSection::Credentials.is_restart_class() returns true
// =========================================================================

#[test]
fn test_credentials_is_restart_class() {
    assert!(
        ConfigSection::Credentials.is_restart_class(),
        "Credentials should be a restart class"
    );
}

#[test]
fn test_models_is_restart_class() {
    assert!(ConfigSection::Models.is_restart_class());
}

#[test]
fn test_channels_is_restart_class() {
    assert!(ConfigSection::Channels.is_restart_class());
}

#[test]
fn test_gateway_is_restart_class() {
    assert!(ConfigSection::Gateway.is_restart_class());
}

#[test]
fn test_system_is_not_restart_class() {
    assert!(
        !ConfigSection::System.is_restart_class(),
        "System should NOT be restart class"
    );
}

#[test]
fn test_session_is_not_restart_class() {
    assert!(!ConfigSection::Session.is_restart_class());
}

#[test]
fn test_plugins_is_not_restart_class() {
    assert!(!ConfigSection::Plugins.is_restart_class());
}

#[test]
fn test_accounts_is_restart_class() {
    assert!(ConfigSection::Accounts.is_restart_class());
}

#[test]
fn test_memory_is_not_restart_class() {
    assert!(!ConfigSection::Memory.is_restart_class());
}

#[test]
fn test_skills_is_not_restart_class() {
    assert!(!ConfigSection::Skills.is_restart_class());
}

#[test]
fn test_media_is_not_restart_class() {
    assert!(!ConfigSection::Media.is_restart_class());
}

// =========================================================================
// Step 1.3 — Credential file permissions
// =========================================================================

/// Permission file with wrong mode → auto-corrected to 0o600.
#[test]
fn test_credential_file_permission_auto_correct() {
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    fs::create_dir_all(&creds_dir).unwrap();
    let cred_file = creds_dir.join("openai.json");
    fs::write(&cred_file, r#"{"provider":"openai","apiKey":"sk-test"}"#).unwrap();

    // Set wrong permissions (world-readable)
    fs::set_permissions(&cred_file, fs::Permissions::from_mode(0o644)).unwrap();
    let mode_before = fs::metadata(&cred_file).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode_before, 0o644, "setup: should be 0o644 before load");

    let provider = CredentialsProvider::load_from_dir(&creds_dir).unwrap();
    assert_eq!(provider.providers.len(), 1);

    // After load, permissions should be corrected to 0o600
    let mode_after = fs::metadata(&cred_file).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode_after, 0o600, "permission should be corrected to 0o600");
}

/// Permission already 0o600 → no modification needed.
#[test]
fn test_credential_file_permission_already_correct() {
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    fs::create_dir_all(&creds_dir).unwrap();
    let cred_file = creds_dir.join("openai.json");
    fs::write(&cred_file, r#"{"provider":"openai","apiKey":"sk-test"}"#).unwrap();

    // Set correct permissions
    fs::set_permissions(&cred_file, fs::Permissions::from_mode(0o600)).unwrap();

    let provider = CredentialsProvider::load_from_dir(&creds_dir).unwrap();
    assert_eq!(provider.providers.len(), 1);

    // Permissions remain 0o600
    let mode = fs::metadata(&cred_file).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o600, "permission should remain 0o600");
}

/// Verify auto-correct from 0o777 → 0o600 on load.
///
/// NOTE: A true `set_permissions` failure scenario (e.g. immutable flag)
/// requires CAP_LINUX_IMMUTABLE which is unavailable in this test
/// environment. The `load_from_dir` implementation emits a WARN but
/// continues loading; that failure path cannot be exercised here.
#[test]
fn test_credential_file_permission_auto_correct_from_0o777() {
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    fs::create_dir_all(&creds_dir).unwrap();
    let cred_file = creds_dir.join("openai.json");
    fs::write(&cred_file, r#"{"provider":"openai","apiKey":"sk-test"}"#).unwrap();

    // Set file to 0o777 (world-readable+executable) and verify auto-correct.
    fs::set_permissions(&cred_file, fs::Permissions::from_mode(0o777)).unwrap();
    let mode_before = fs::metadata(&cred_file).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode_before, 0o777, "setup: should be 0o777 before load");

    let provider = CredentialsProvider::load_from_dir(&creds_dir).unwrap();
    assert_eq!(provider.providers.len(), 1, "credential should be loaded");
    assert!(provider.get_api_key("openai").is_some());

    // After load, permissions should be corrected to 0o600.
    let mode_after = fs::metadata(&cred_file).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode_after, 0o600, "permission should be corrected to 0o600");
}

/// Multiple credential files: mixed permissions all get corrected.
#[test]
fn test_credential_files_multiple_mixed_permissions() {
    let tmp = TempDir::new().unwrap();
    let creds_dir = tmp.path().join("credentials");
    fs::create_dir_all(&creds_dir).unwrap();

    // File 1: 0o644 (needs correction)
    let f1 = creds_dir.join("openai.json");
    fs::write(&f1, r#"{"provider":"openai","apiKey":"sk-oai"}"#).unwrap();
    fs::set_permissions(&f1, fs::Permissions::from_mode(0o644)).unwrap();

    // File 2: 0o600 (already correct)
    let f2 = creds_dir.join("anthropic.json");
    fs::write(&f2, r#"{"provider":"anthropic","apiKey":"sk-ant"}"#).unwrap();
    fs::set_permissions(&f2, fs::Permissions::from_mode(0o600)).unwrap();

    // File 3: 0o666 (needs correction)
    let f3 = creds_dir.join("custom.json");
    fs::write(&f3, r#"{"provider":"custom","apiKey":"sk-custom"}"#).unwrap();
    fs::set_permissions(&f3, fs::Permissions::from_mode(0o666)).unwrap();

    let provider = CredentialsProvider::load_from_dir(&creds_dir).unwrap();
    assert_eq!(provider.providers.len(), 3);

    // All files should now be 0o600
    for name in &["openai", "anthropic", "custom"] {
        let cred_file = creds_dir.join(format!("{}.json", name));
        let mode = fs::metadata(&cred_file).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600, "{} should be 0o600", name);
    }
}

// =========================================================================
// Step 1.4 — accounts.json bindings deserialization + query
// =========================================================================

#[test]
fn test_accounts_json_with_bindings_deserializes() {
    let json = r#"{
        "accounts": [
            {"platform":"feishu","sender_id":"ou_a","account_id":"a1"}
        ],
        "bindings": [
            {"bot_app_id":"app1","agent_id":"eda"},
            {"bot_app_id":"app2","agent_id":"ghost"}
        ]
    }"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert_eq!(data.bindings.len(), 2);
    assert_eq!(data.bindings[0].bot_app_id, "app1");
    assert_eq!(data.bindings[0].agent_id, "eda");
    assert_eq!(data.bindings[1].bot_app_id, "app2");
    assert_eq!(data.bindings[1].agent_id, "ghost");
}

#[test]
fn test_accounts_json_empty_bindings_defaults_empty_vec() {
    let json = r#"{"accounts": [], "bindings": []}"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert!(data.bindings.is_empty());
}

#[test]
fn test_accounts_json_missing_bindings_key_defaults_empty_vec() {
    let json = r#"{"accounts": []}"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert!(data.bindings.is_empty());
}

#[test]
fn test_accounts_json_bindings_serde_roundtrip() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![
            BotAgentBinding {
                bot_app_id: "app1".to_string(),
                agent_id: "eda".to_string(),
            },
            BotAgentBinding {
                bot_app_id: "app2".to_string(),
                agent_id: "ghost".to_string(),
            },
        ],
    };
    let json = serde_json::to_string(&data).unwrap();
    let restored: AccountsConfigData = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.bindings, data.bindings);
}

#[test]
fn test_accounts_get_binding_hit() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![BotAgentBinding {
            bot_app_id: "app1".to_string(),
            agent_id: "eda".to_string(),
        }],
    };
    let b = data.get_binding("app1").unwrap();
    assert_eq!(b.agent_id, "eda");
}

#[test]
fn test_accounts_get_binding_miss() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![BotAgentBinding {
            bot_app_id: "app1".to_string(),
            agent_id: "eda".to_string(),
        }],
    };
    assert!(data.get_binding("nonexistent").is_none());
}

#[test]
fn test_accounts_get_binding_empty_bindings() {
    let data = AccountsConfigData::default();
    assert!(data.get_binding("app1").is_none());
}

#[test]
fn test_accounts_bindings_validation_pass() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![BotAgentBinding {
            bot_app_id: "app1".to_string(),
            agent_id: "eda".to_string(),
        }],
    };
    assert!(data.validate().is_ok());
}

// =========================================================================
// Step 1.5 — system.json: version empty → error, cron invalid → error, valid → pass
// =========================================================================

#[test]
fn test_validate_system_empty_version_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":""}"#).unwrap();
    let err = for_section(ConfigSection::System)(&v).unwrap_err();
    assert!(
        err.contains("version cannot be an empty string"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_system_version_number_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":123}"#).unwrap();
    let err = for_section(ConfigSection::System)(&v).unwrap_err();
    assert!(err.contains("version must be a string"), "error: {}", err);
}

#[test]
fn test_validate_system_valid_version_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":"1.0.0"}"#).unwrap();
    assert!(for_section(ConfigSection::System)(&v).is_ok());
}

#[test]
fn test_validate_system_invalid_cron_returns_error() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"cron":{"schedule":"not a cron expr"}}"#).unwrap();
    let err = for_section(ConfigSection::System)(&v).unwrap_err();
    assert!(
        err.contains("system.cron.schedule must be a valid cron expression"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_system_valid_cron_passes() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"cron":{"schedule":"0 */6 * * * *"}}"#).unwrap();
    assert!(for_section(ConfigSection::System)(&v).is_ok());
}

#[test]
fn test_validate_system_empty_cron_string_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"{"cron":{"schedule":""}}"#).unwrap();
    assert!(for_section(ConfigSection::System)(&v).is_ok());
}

#[test]
fn test_validate_system_cron_not_string_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"{"cron":{"schedule":123}}"#).unwrap();
    let err = for_section(ConfigSection::System)(&v).unwrap_err();
    assert!(
        err.contains("system.cron.schedule must be a string"),
        "error: {}",
        err
    );
}

// =========================================================================
// Step 1.6 — credentials: provider empty → error, apiKey empty → error, valid → pass
// =========================================================================

#[test]
fn test_validate_credentials_empty_provider_returns_error() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"provider":"","apiKey":"sk-test"}"#).unwrap();
    let err = validate_credentials(&v).unwrap_err();
    assert!(
        err.contains("credentials.provider cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_credentials_missing_provider_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"{"apiKey":"sk-test"}"#).unwrap();
    let err = validate_credentials(&v).unwrap_err();
    assert!(
        err.contains("credentials.provider is required"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_credentials_empty_api_key_returns_error() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"provider":"openai","apiKey":""}"#).unwrap();
    let err = validate_credentials(&v).unwrap_err();
    assert!(
        err.contains("credentials.apiKey cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_credentials_valid_api_key_passes() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"provider":"openai","apiKey":"sk-test123"}"#).unwrap();
    assert!(validate_credentials(&v).is_ok());
}

#[test]
fn test_validate_credentials_valid_feishu_passes() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"provider":"feishu","appId":"cli_abc","appSecret":"secret"}"#)
            .unwrap();
    assert!(validate_credentials(&v).is_ok());
}

#[test]
fn test_validate_credentials_empty_app_id_returns_error() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"provider":"feishu","appId":"","appSecret":"secret"}"#).unwrap();
    let err = validate_credentials(&v).unwrap_err();
    assert!(
        err.contains("credentials.appId cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_credentials_empty_app_secret_returns_error() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"provider":"feishu","appId":"cli_abc","appSecret":""}"#).unwrap();
    let err = validate_credentials(&v).unwrap_err();
    assert!(
        err.contains("credentials.appSecret cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_credentials_not_object_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"null"#).unwrap();
    let err = validate_credentials(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_credentials_null_api_key_passes() {
    // apiKey: null should not fail (absent/null is fine)
    let v: serde_json::Value =
        serde_json::from_str(r#"{"provider":"openai","apiKey":null}"#).unwrap();
    assert!(validate_credentials(&v).is_ok());
}

#[test]
fn test_validate_credentials_missing_api_key_passes() {
    // apiKey not present → should pass (only checked if present)
    let v: serde_json::Value = serde_json::from_str(r#"{"provider":"openai"}"#).unwrap();
    assert!(validate_credentials(&v).is_ok());
}

// =========================================================================
// Step 1.7 — skills: empty path → error, null byte → error, valid → pass
// =========================================================================

#[test]
fn test_validate_skills_empty_path_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"{"extraDirs":[""]}"#).unwrap();
    let err = for_section(ConfigSection::Skills)(&v).unwrap_err();
    assert!(err.contains("cannot be an empty path"), "error: {}", err);
}

#[test]
fn test_validate_skills_null_byte_returns_error() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"extraDirs":["/some/path\u0000/here"]}"#).unwrap();
    let err = for_section(ConfigSection::Skills)(&v).unwrap_err();
    assert!(err.contains("contains a null byte"), "error: {}", err);
}

#[test]
fn test_validate_skills_valid_path_passes() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"extraDirs":["/home/user/skills","./relative"]}"#).unwrap();
    assert!(for_section(ConfigSection::Skills)(&v).is_ok());
}

#[test]
fn test_validate_skills_absent_extra_dirs_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(for_section(ConfigSection::Skills)(&v).is_ok());
}

#[test]
fn test_validate_skills_empty_extra_dirs_array_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"{"extraDirs":[]}"#).unwrap();
    assert!(for_section(ConfigSection::Skills)(&v).is_ok());
}

#[test]
fn test_validate_skills_non_string_in_array_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"{"extraDirs":[123]}"#).unwrap();
    let err = for_section(ConfigSection::Skills)(&v).unwrap_err();
    assert!(err.contains("must be a string"), "error: {}", err);
}

#[test]
fn test_validate_skills_not_object_returns_error() {
    let v: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let err = for_section(ConfigSection::Skills)(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_skills_multiple_valid_paths_pass() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"extraDirs":["/opt/skills","~/my-skills","../shared-skills"]}"#)
            .unwrap();
    assert!(for_section(ConfigSection::Skills)(&v).is_ok());
}

#[test]
fn test_validate_skills_one_empty_one_valid_fails() {
    let v: serde_json::Value = serde_json::from_str(r#"{"extraDirs":["/valid",""]}"#).unwrap();
    let err = for_section(ConfigSection::Skills)(&v).unwrap_err();
    assert!(err.contains("cannot be an empty path"), "error: {}", err);
}

// =========================================================================
// Cross-cutting: for_section returns correct validators
// =========================================================================

#[test]
fn test_for_section_credentials_returns_validate_credentials() {
    let validator = for_section(ConfigSection::Credentials);
    let valid: serde_json::Value =
        serde_json::from_str(r#"{"provider":"openai","apiKey":"sk-test"}"#).unwrap();
    assert!(validator(&valid).is_ok());
    let invalid: serde_json::Value = serde_json::from_str(r#"null"#).unwrap();
    assert!(validator(&invalid).is_err());
}

#[test]
fn test_for_section_skills_returns_validate_skills() {
    let validator = for_section(ConfigSection::Skills);
    let valid: serde_json::Value = serde_json::from_str(r#"{"extraDirs":["/path"]}"#).unwrap();
    assert!(validator(&valid).is_ok());
    let invalid: serde_json::Value = serde_json::from_str(r#"{"extraDirs":[""]}"#).unwrap();
    assert!(validator(&invalid).is_err());
}

#[test]
fn test_for_section_system_returns_validate_system() {
    let validator = for_section(ConfigSection::System);
    let valid: serde_json::Value = serde_json::from_str(r#"{"version":"1.0"}"#).unwrap();
    assert!(validator(&valid).is_ok());
    let invalid: serde_json::Value = serde_json::from_str(r#"{"version":""}"#).unwrap();
    assert!(validator(&invalid).is_err());
}
