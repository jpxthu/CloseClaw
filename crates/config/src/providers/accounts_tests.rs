//! Unit tests for AccountsConfigData and accounts validator.

use crate::identity::IdentityMapping;
use crate::manager::ConfigSection;
use crate::providers::accounts::{AccountsConfigData, BotAgentBinding};
use crate::providers::ConfigProvider;
use crate::validators::for_section;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_account(platform: &str, sender_id: &str, account_id: &str) -> IdentityMapping {
    IdentityMapping {
        platform: platform.to_string(),
        bot_app_id: String::new(),
        sender_id: sender_id.to_string(),
        account_id: account_id.to_string(),
    }
}

fn make_account_with_bot(
    platform: &str,
    bot_app_id: &str,
    sender_id: &str,
    account_id: &str,
) -> IdentityMapping {
    IdentityMapping {
        platform: platform.to_string(),
        bot_app_id: bot_app_id.to_string(),
        sender_id: sender_id.to_string(),
        account_id: account_id.to_string(),
    }
}

fn sample_accounts_data() -> AccountsConfigData {
    AccountsConfigData {
        accounts: vec![
            make_account("feishu", "ou_aaa", "local_user_1"),
            make_account("discord", "12345", "local_user_2"),
            make_account("telegram", "@user1", "local_user_3"),
        ],
        bindings: vec![],
    }
}

// ---------------------------------------------------------------------------
// Normal path
// ---------------------------------------------------------------------------

#[test]
fn test_valid_accounts_pass_validation() {
    let data = sample_accounts_data();
    assert!(data.validate().is_ok());
}

#[test]
fn test_load_from_json_str_valid() {
    let json = r#"{
        "accounts": [
            {"platform":"feishu","sender_id":"ou_aaa","account_id":"u1"},
            {"platform":"discord","sender_id":"12345","account_id":"u2"}
        ]
    }"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert_eq!(data.accounts.len(), 2);
    assert!(data.validate().is_ok());
}

#[test]
fn test_get_account_hit() {
    let data = sample_accounts_data();
    let acc = data.get_account("local_user_1").unwrap();
    assert_eq!(acc.platform, "feishu");
    assert_eq!(acc.sender_id, "ou_aaa");
}

#[test]
fn test_get_account_miss() {
    let data = sample_accounts_data();
    assert!(data.get_account("nonexistent").is_none());
}

#[test]
fn test_accounts_by_platform() {
    let data = sample_accounts_data();
    let feishu = data.accounts_by_platform("feishu");
    assert_eq!(feishu.len(), 1);
    assert_eq!(feishu[0].account_id, "local_user_1");
}

#[test]
fn test_accounts_by_platform_empty() {
    let data = sample_accounts_data();
    let slack = data.accounts_by_platform("slack");
    assert!(slack.is_empty());
}

#[test]
fn test_version() {
    let data = sample_accounts_data();
    assert_eq!(data.version(), "1.0.0");
}

#[test]
fn test_config_path() {
    assert_eq!(AccountsConfigData::config_path(), "accounts.json");
}

// ---------------------------------------------------------------------------
// Error path — validation
// ---------------------------------------------------------------------------

#[test]
fn test_validate_fail_empty_account_id() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![make_account("feishu", "ou_aaa", "")],
    };
    let err = data.validate().unwrap_err();
    assert!(err.to_string().contains("account_id cannot be empty"));
}

#[test]
fn test_validate_fail_empty_sender_id() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![make_account("feishu", "", "local_user_1")],
    };
    let err = data.validate().unwrap_err();
    assert!(err.to_string().contains("sender_id cannot be empty"));
}

#[test]
fn test_validate_fail_duplicate_account_id() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account("feishu", "ou_aaa", "user1"),
            make_account("discord", "12345", "user1"),
        ],
    };
    let err = data.validate().unwrap_err();
    assert!(
        err.to_string().contains("duplicate account_id"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_fail_duplicate_reported_index() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account("feishu", "ou_aaa", "user1"),
            make_account("discord", "12345", "user1"),
            make_account("telegram", "@u", "user2"),
        ],
    };
    let err = data.validate().unwrap_err();
    assert!(err.to_string().contains("at index 1"), "error: {}", err);
}

#[test]
fn test_load_invalid_json() {
    let err = AccountsConfigData::from_json_str("not json").unwrap_err();
    // Should be a JSON parse error
    assert!(err.to_string().contains("JSON") || err.to_string().contains("expected"));
}

// ---------------------------------------------------------------------------
// Boundary values
// ---------------------------------------------------------------------------

#[test]
fn test_empty_accounts_list_is_default() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![],
    };
    assert!(data.is_default());
    assert!(data.validate().is_ok());
}

#[test]
fn test_empty_accounts_json_is_default() {
    let json = r#"{"accounts":[]}"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert!(data.is_default());
}

#[test]
fn test_single_account() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![make_account("feishu", "ou_123", "acc_1")],
    };
    assert!(!data.is_default());
    assert!(data.validate().is_ok());
}

#[test]
fn test_multiple_accounts() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account("feishu", "ou_1", "acc_1"),
            make_account("discord", "d_1", "acc_2"),
            make_account("telegram", "t_1", "acc_3"),
        ],
    };
    assert!(!data.is_default());
    assert_eq!(data.accounts.len(), 3);
    assert!(data.validate().is_ok());
}

#[test]
fn test_missing_accounts_key_defaults_empty() {
    let json = r#"{}"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert!(data.accounts.is_empty());
    assert!(data.is_default());
    assert!(data.validate().is_ok());
}

// ---------------------------------------------------------------------------
// Serialization round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_roundtrip_json() {
    let data = sample_accounts_data();
    let json = serde_json::to_string(&data).unwrap();
    let restored: AccountsConfigData = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.accounts.len(), data.accounts.len());
    for (a, b) in data.accounts.iter().zip(restored.accounts.iter()) {
        assert_eq!(a.platform, b.platform);
        assert_eq!(a.sender_id, b.sender_id);
        assert_eq!(a.account_id, b.account_id);
    }
}

#[test]
fn test_roundtrip_pretty() {
    let data = sample_accounts_data();
    let json = serde_json::to_string_pretty(&data).unwrap();
    let restored: AccountsConfigData = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.accounts.len(), 3);
    assert!(restored.validate().is_ok());
}

// ---------------------------------------------------------------------------
// Validator integration — for_section(ConfigSection::Accounts)
// ---------------------------------------------------------------------------

#[test]
fn test_for_section_accounts_returns_validator() {
    let validator = for_section(ConfigSection::Accounts);
    let valid: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[{"platform":"feishu","senderId":"ou_a","accountId":"a1"}]}"#,
    )
    .unwrap();
    assert!(validator(&valid).is_ok());
}

#[test]
fn test_for_section_accounts_rejects_non_object() {
    let validator = for_section(ConfigSection::Accounts);
    let invalid: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    assert!(validator(&invalid).is_err());
}

#[test]
fn test_accounts_validator_pass_empty_accounts_array() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(r#"{"accounts":[]}"#).unwrap();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_accounts_validator_pass_no_accounts_key() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_accounts_validator_fail_empty_account_id() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[{"platform":"feishu","senderId":"ou_a","accountId":""}]}"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("accountId cannot be empty"), "error: {}", err);
}

#[test]
fn test_accounts_validator_fail_empty_sender_id() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[{"platform":"feishu","senderId":"","accountId":"a1"}]}"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("senderId cannot be empty"), "error: {}", err);
}

#[test]
fn test_accounts_validator_fail_duplicate_account_id() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"
            {"accounts":[
                {"platform":"feishu","senderId":"ou_a","accountId":"a1"},
                {"platform":"discord","senderId":"d_1","accountId":"a1"}
            ]}
        "#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("duplicate accountId"), "error: {}", err);
}

#[test]
fn test_accounts_validator_fail_unknown_platform() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[{"platform":"unknown_platform","senderId":"ou_a","accountId":"a1"}]}"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("not a known channel type"), "error: {}", err);
}

#[test]
fn test_accounts_validator_pass_valid_platforms() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[
            {"platform":"feishu","senderId":"ou_a","accountId":"a1"},
            {"platform":"discord","senderId":"d_1","accountId":"a2"},
            {"platform":"telegram","senderId":"t_1","accountId":"a3"},
            {"platform":"slack","senderId":"s_1","accountId":"a4"},
            {"platform":"whatsapp","senderId":"w_1","accountId":"a5"},
            {"platform":"signal","senderId":"sig_1","accountId":"a6"},
            {"platform":"matrix","senderId":"m_1","accountId":"a7"},
            {"platform":"msteams","senderId":"ms_1","accountId":"a8"},
            {"platform":"mattermost","senderId":"mm_1","accountId":"a9"},
            {"platform":"nostr","senderId":"n_1","accountId":"a10"}
        ]}"#,
    )
    .unwrap();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_accounts_passes() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[{"platform":"feishu","senderId":"ou_a","accountId":"a1"}]}"#,
    )
    .unwrap();
    let validator = ConfigSection::Accounts.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_accounts_rejects_non_object() {
    let v: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let validator = ConfigSection::Accounts.default_validator();
    assert!(validator(&v).is_err());
}

fn make_binding(bot_app_id: &str, agent_id: &str) -> BotAgentBinding {
    BotAgentBinding {
        bot_app_id: bot_app_id.to_string(),
        agent_id: agent_id.to_string(),
    }
}

// ---------------------------------------------------------------------------
// BotAgentBinding validation — via validator (Step 1.11)
// ---------------------------------------------------------------------------

#[test]
fn test_accounts_validator_pass_with_valid_bindings() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{
            "accounts":[{"platform":"feishu","senderId":"ou_a","accountId":"a1"}],
            "bindings":[{"bot_app_id":"app1","agent_id":"eda"}]
        }"#,
    )
    .unwrap();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_accounts_validator_pass_multiple_unique_bindings() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{
            "accounts":[],
            "bindings":[
                {"bot_app_id":"app1","agent_id":"eda"},
                {"bot_app_id":"app2","agent_id":"ghost"}
            ]
        }"#,
    )
    .unwrap();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_accounts_validator_fail_empty_bot_app_id_in_binding() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{
            "accounts":[],
            "bindings":[{"bot_app_id":"","agent_id":"eda"}]
        }"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("bot_app_id cannot be empty"), "error: {}", err);
}

#[test]
fn test_accounts_validator_fail_empty_agent_id_in_binding() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{
            "accounts":[],
            "bindings":[{"bot_app_id":"app1","agent_id":""}]
        }"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("agent_id cannot be empty"), "error: {}", err);
}

#[test]
fn test_accounts_validator_fail_duplicate_bot_app_id() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{
            "accounts":[],
            "bindings":[
                {"bot_app_id":"app1","agent_id":"eda"},
                {"bot_app_id":"app1","agent_id":"ghost"}
            ]
        }"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(
        err.contains("bot_app_id 'app1' is not unique"),
        "error: {}",
        err
    );
}

#[test]
fn test_accounts_validator_fail_bindings_not_array() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value =
        serde_json::from_str(r#"{"accounts":[],"bindings":"invalid"}"#).unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("must be a JSON array"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// BotAgentBinding — normal path
// ---------------------------------------------------------------------------

#[test]
fn test_load_with_bindings() {
    let json = r#"{
        "accounts": [
            {"platform":"feishu","sender_id":"ou_a","account_id":"a1"}
        ],
        "bindings": [
            {"bot_app_id":"app1","agent_id":"eda"}
        ]
    }"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert_eq!(data.bindings.len(), 1);
    assert_eq!(data.bindings[0].bot_app_id, "app1");
    assert_eq!(data.bindings[0].agent_id, "eda");
}

#[test]
fn test_load_empty_bindings_defaults_empty() {
    let json = r#"{ "accounts": [] }"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert!(data.bindings.is_empty());
}

#[test]
fn test_load_missing_bindings_key_defaults_empty() {
    let json = r#"{}"#;
    let data = AccountsConfigData::from_json_str(json).unwrap();
    assert!(data.bindings.is_empty());
}

#[test]
fn test_get_binding_hit() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![make_binding("app1", "eda")],
    };
    let b = data.get_binding("app1").unwrap();
    assert_eq!(b.agent_id, "eda");
}

#[test]
fn test_get_binding_miss() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![make_binding("app1", "eda")],
    };
    assert!(data.get_binding("app_unknown").is_none());
}

#[test]
fn test_get_binding_empty_bindings() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![],
    };
    assert!(data.get_binding("app1").is_none());
}

#[test]
fn test_bindings_serde_roundtrip() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![make_binding("app1", "eda"), make_binding("app2", "ghost")],
    };
    let json = serde_json::to_string(&data).unwrap();
    let restored: AccountsConfigData = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.bindings.len(), 2);
    assert_eq!(restored.bindings[0], data.bindings[0]);
    assert_eq!(restored.bindings[1], data.bindings[1]);
}

#[test]
fn test_is_default_empty_bindings() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![],
    };
    assert!(data.is_default());
}

#[test]
fn test_is_default_with_bindings() {
    let data = AccountsConfigData {
        accounts: vec![],
        bindings: vec![make_binding("app1", "eda")],
    };
    assert!(!data.is_default());
}

#[test]
fn test_is_default_with_accounts_only() {
    let data = AccountsConfigData {
        accounts: vec![make_account("feishu", "ou_aaa", "u1")],
        bindings: vec![],
    };
    assert!(!data.is_default());
}

// ---------------------------------------------------------------------------
// Validator integration — bindings via for_section
// ---------------------------------------------------------------------------

#[test]
fn test_accounts_validator_pass_with_bindings() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{
            "accounts":[{"platform":"feishu","senderId":"ou_a","accountId":"a1"}],
            "bindings":[{"bot_app_id":"app1","agent_id":"eda"}]
        }"#,
    )
    .unwrap();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_accounts_validator_pass_empty_bindings() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(r#"{"accounts":[],"bindings":[]}"#).unwrap();
    assert!(validator(&v).is_ok());
}

// ---------------------------------------------------------------------------
// Bot app ID × sender_id uniqueness constraint
// ---------------------------------------------------------------------------

#[test]
fn test_validate_fail_duplicate_platform_binding() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account("feishu", "ou_aaa", "user1"),
            make_account("feishu", "ou_aaa", "user2"),
        ],
    };
    let err = data.validate().unwrap_err();
    assert!(
        err.to_string().contains("duplicate binding"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_fail_duplicate_platform_binding_with_bot_app_id() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account_with_bot("feishu", "app1", "ou_aaa", "user1"),
            make_account_with_bot("feishu", "app1", "ou_aaa", "user2"),
        ],
    };
    let err = data.validate().unwrap_err();
    assert!(
        err.to_string().contains("duplicate binding"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_pass_different_bot_app_id_same_sender() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account_with_bot("feishu", "app1", "ou_aaa", "user1"),
            make_account_with_bot("feishu", "app2", "ou_aaa", "user2"),
        ],
    };
    assert!(data.validate().is_ok());
}

#[test]
fn test_validate_pass_same_sender_different_platform() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account("feishu", "ou_aaa", "user1"),
            make_account("discord", "ou_aaa", "user2"),
        ],
    };
    assert!(data.validate().is_ok());
}

#[test]
fn test_validate_fail_duplicate_binding_error_contains_index() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account("feishu", "ou_aaa", "user1"),
            make_account("feishu", "ou_aaa", "user2"),
            make_account("feishu", "ou_bbb", "user3"),
        ],
    };
    let err = data.validate().unwrap_err();
    assert!(err.to_string().contains("at index 1"), "error: {}", err);
}

#[test]
fn test_validate_pass_unique_bindings() {
    let data = AccountsConfigData {
        bindings: vec![],
        accounts: vec![
            make_account_with_bot("feishu", "app1", "ou_aaa", "user1"),
            make_account_with_bot("feishu", "app1", "ou_bbb", "user2"),
            make_account_with_bot("feishu", "app2", "ou_aaa", "user3"),
            make_account("discord", "d_1", "user4"),
        ],
    };
    assert!(data.validate().is_ok());
}

// ---------------------------------------------------------------------------
// Validator integration — binding uniqueness via for_section
// ---------------------------------------------------------------------------

#[test]
fn test_accounts_validator_fail_duplicate_binding() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[
            {"platform":"feishu","senderId":"ou_a","accountId":"a1"},
            {"platform":"feishu","senderId":"ou_a","accountId":"a2"}
        ]}"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("duplicate binding"), "error: {}", err);
}

#[test]
fn test_accounts_validator_pass_different_bot_app_id_same_sender() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[
            {"platform":"feishu","botAppId":"app1","senderId":"ou_a","accountId":"a1"},
            {"platform":"feishu","botAppId":"app2","senderId":"ou_a","accountId":"a2"}
        ]}"#,
    )
    .unwrap();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_accounts_validator_fail_duplicate_with_bot_app_id() {
    let validator = for_section(ConfigSection::Accounts);
    let v: serde_json::Value = serde_json::from_str(
        r#"{"accounts":[
            {"platform":"feishu","botAppId":"app1","senderId":"ou_a","accountId":"a1"},
            {"platform":"feishu","botAppId":"app1","senderId":"ou_a","accountId":"a2"}
        ]}"#,
    )
    .unwrap();
    let err = validator(&v).unwrap_err();
    assert!(err.contains("duplicate binding"), "error: {}", err);
}
