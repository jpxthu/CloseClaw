//! Tests for config section validators.

use crate::manager::ConfigSection;
use crate::validators::{
    for_section, validate_channels, validate_gateway, validate_models, validate_plugins,
    validate_session, validate_system,
};

// ---------------------------------------------------------------------------
// validate_models
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_pass() {
    let v: serde_json::Value = serde_json::from_str(r#"{"models":[{"id":"m1"}]}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_pass_empty_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_pass_with_array() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[{"id":"m1"}]}}}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_fail_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#""string""#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_models_fail_array_top_level() {
    let v: serde_json::Value = serde_json::from_str(r#"[1,2,3]"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_models_fail_models_not_array() {
    let v: serde_json::Value = serde_json::from_str(r#"{"models":"not array"}"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("array"), "error: {}", err);
}

#[test]
fn test_validate_models_fail_empty_provider_id() {
    let v: serde_json::Value = serde_json::from_str(r#"{"providers":{"":{"models":[]}}}"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(
        err.contains("provider ID cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_models_fail_empty_model_id() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[{"id":""}]}}}"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("id cannot be empty"), "error: {}", err);
}

#[test]
fn test_validate_models_fail_missing_model_id() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[{"name":"no-id"}]}}}"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("id is required"), "error: {}", err);
}

#[test]
fn test_validate_models_fail_invalid_base_url() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"baseUrl":"ftp://bad","models":[]}}}"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("baseUrl must start with"), "error: {}", err);
}

#[test]
fn test_validate_models_pass_valid_base_url() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"providers":{"p":{"baseUrl":"https://api.example.com","models":[]}}}"#,
    )
    .unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_pass_empty_base_url() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"baseUrl":"","models":[]}}}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_pass_multiple_providers() {
    let v: serde_json::Value = serde_json::from_str(
        r#"
        {
            "providers": {
                "openai": {
                    "baseUrl": "https://api.openai.com",
                    "models": [{"id": "gpt-4"}]
                },
                "anthropic": {
                    "baseUrl": "https://api.anthropic.com",
                    "models": [{"id": "claude-3"}]
                }
            }
        }
        "#,
    )
    .unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_fail_model_not_object() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":["bad"]}}}"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("must be objects"), "error: {}", err);
}

#[test]
fn test_validate_models_fail_provider_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"providers":{"p":"not-object"}}"#).unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("must be a JSON object"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// validate_channels
// ---------------------------------------------------------------------------

#[test]
fn test_validate_channels_pass() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":"acc1"}}]}"#,
    )
    .unwrap();
    assert!(validate_channels(&v).is_ok());
}

#[test]
fn test_validate_channels_pass_empty_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_channels(&v).is_ok());
}

#[test]
fn test_validate_channels_fail_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_channels_not_array() {
    let v: serde_json::Value = serde_json::from_str(r#"{"channels":"bad"}"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_unknown_type() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"channels":{"unknown-channel":{"enabled":true}}}"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("unknown channel type"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_empty_type_key() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"channels":{"":{"enabled":true}}}"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("cannot be empty"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_bindings_not_array() {
    let v: serde_json::Value = serde_json::from_str(r#"{"bindings":"not-array"}"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("array"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_binding_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"bindings":["bad"]}"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("must be a JSON object"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_binding_missing_agent_id() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"bindings":[{"match":{"channel":"feishu","accountId":"a"}}]}"#)
            .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("agentId is required"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_binding_empty_agent_id() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"bindings":[{"agentId":"","match":{"channel":"feishu","accountId":"a"}}]}"#,
    )
    .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("agentId cannot be empty"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_binding_missing_match() {
    let v: serde_json::Value = serde_json::from_str(r#"{"bindings":[{"agentId":"a1"}]}"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("match is required"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_binding_match_not_object() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"bindings":[{"agentId":"a1","match":"bad"}]}"#).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(
        err.contains("match must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_channels_fail_binding_missing_channel() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"bindings":[{"agentId":"a1","match":{"accountId":"a"}}]}"#)
            .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(err.contains("match.channel is required"), "error: {}", err);
}

#[test]
fn test_validate_channels_fail_binding_empty_channel() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"bindings":[{"agentId":"a1","match":{"channel":"","accountId":"a"}}]}"#,
    )
    .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(
        err.contains("match.channel cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_channels_fail_binding_missing_account_id() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu"}}]}"#,
    )
    .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(
        err.contains("match.accountId is required"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_channels_fail_binding_empty_account_id() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":""}}]}"#,
    )
    .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(
        err.contains("match.accountId cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_channels_pass_multiple_valid() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true},"telegram":{"enabled":false}},"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":"acc1"}},{"agentId":"a2","match":{"channel":"telegram","accountId":"bot1"}}]}"#,
    )
    .unwrap();
    assert!(validate_channels(&v).is_ok());
}

#[test]
fn test_validate_channels_pass_no_bindings() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"channels":{"feishu":{"enabled":true}}}"#).unwrap();
    assert!(validate_channels(&v).is_ok());
}

// ---------------------------------------------------------------------------
// validate_gateway
// ---------------------------------------------------------------------------

#[test]
fn test_validate_gateway_pass() {
    let v: serde_json::Value = serde_json::from_str(r#"{"port":8080}"#).unwrap();
    assert!(validate_gateway(&v).is_ok());
}

#[test]
fn test_validate_gateway_pass_empty() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_gateway(&v).is_ok());
}

#[test]
fn test_validate_gateway_fail_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"123"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_gateway_fail_port_and_timeout() {
    // port out of range
    let v: serde_json::Value = serde_json::from_str(r#"{"port":0}"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(err.contains("range 1-65535"), "error: {}", err);
    // port too high
    let v: serde_json::Value = serde_json::from_str(r#"{"port":99999}"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(err.contains("range 1-65535"), "error: {}", err);
    // port string
    let v: serde_json::Value = serde_json::from_str(r#"{"port":"abc"}"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(err.contains("non-negative integer"), "error: {}", err);
    // negative port
    let v: serde_json::Value = serde_json::from_str(r#"{"port":-1}"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(err.contains("non-negative integer"), "error: {}", err);
    // timeout negative
    let v: serde_json::Value = serde_json::from_str(r#"{"timeout":-1000}"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(err.contains("non-negative"), "error: {}", err);
    // timeout string
    let v: serde_json::Value = serde_json::from_str(r#"{"timeout":"not-a-number"}"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(err.contains("must be a number"), "error: {}", err);
}

#[test]
fn test_validate_gateway_pass_valid_timeout() {
    let v: serde_json::Value = serde_json::from_str(r#"{"timeout":30000}"#).unwrap();
    assert!(validate_gateway(&v).is_ok());
}

#[test]
fn test_validate_gateway_pass_zero_timeout() {
    let v: serde_json::Value = serde_json::from_str(r#"{"timeout":0}"#).unwrap();
    assert!(validate_gateway(&v).is_ok());
}

#[test]
fn test_validate_gateway_pass_all_valid_fields() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"port":8080,"timeout":30000,"name":"gw"}"#).unwrap();
    assert!(validate_gateway(&v).is_ok());
}

// ---------------------------------------------------------------------------
// validate_plugins
// ---------------------------------------------------------------------------

#[test]
fn test_validate_plugins_pass_empty_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_fail_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"null"#).unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_plugins_pass_with_entries() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"entries":{"minimax":{"enabled":true},"lark":{"enabled":false}}}"#,
    )
    .unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_fail_empty_entry_name() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"entries":{"":{"enabled":true}}}"#).unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(
        err.contains("plugin name cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_plugins_pass_with_allow() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"allow":["minimax","openclaw-lark"]}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_fail_empty_allow_name() {
    let v: serde_json::Value = serde_json::from_str(r#"{"allow":["minimax",""]}"#).unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(
        err.contains("plugins.allow[1] plugin name cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_plugins_fail_allow_not_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"allow":[123]}"#).unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(err.contains("must be a string"), "error: {}", err);
}

#[test]
fn test_validate_plugins_pass_with_installs() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"installs":{"openclaw-lark":{"source":"archive"}}}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_fail_empty_install_name() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"installs":{"":{"source":"archive"}}}"#).unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(
        err.contains("plugin name cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_plugins_fail_install_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"installs":{"p":"not-object"}}"#).unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(err.contains("must be a JSON object"), "error: {}", err);
}

#[test]
fn test_validate_plugins_fail_install_path_not_exists() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"installs":{"p":{"installPath":"/nonexistent/path/to/plugin"}}}"#)
            .unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(err.contains("does not exist"), "error: {}", err);
}

#[test]
fn test_validate_plugins_pass_install_path_empty_string() {
    // Empty installPath should be ignored
    let v: serde_json::Value =
        serde_json::from_str(r#"{"installs":{"p":{"installPath":""}}}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_pass_install_path_absent() {
    // No installPath field at all
    let v: serde_json::Value =
        serde_json::from_str(r#"{"installs":{"p":{"source":"archive"}}}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_pass_all_fields() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"version":"1.0.0","enabled":true,"allow":["minimax"],"entries":{"minimax":{"enabled":true}},"installs":{"minimax":{"source":"archive"}}}"#,
    )
    .unwrap();
    assert!(validate_plugins(&v).is_ok());
}

// ---------------------------------------------------------------------------
// validate_system
// ---------------------------------------------------------------------------

#[test]
fn test_validate_system_pass() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":"1.0"}"#).unwrap();
    assert!(validate_system(&v).is_ok());
}

#[test]
fn test_validate_system_pass_empty() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_system(&v).is_ok());
}

#[test]
fn test_validate_system_fail_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"[true]"#).unwrap();
    let err = validate_system(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// validate_system — version
// ---------------------------------------------------------------------------

#[test]
fn test_validate_system_pass_version_non_empty() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":"1.0"}"#).unwrap();
    assert!(validate_system(&v).is_ok());
}

#[test]
fn test_validate_system_pass_version_absent() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_system(&v).is_ok());
}

#[test]
fn test_validate_system_fail_version_empty_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":""}"#).unwrap();
    let err = validate_system(&v).unwrap_err();
    assert!(
        err.contains("version cannot be an empty string"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_system_fail_version_not_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":123}"#).unwrap();
    let err = validate_system(&v).unwrap_err();
    assert!(err.contains("version must be a string"), "error: {}", err);
}

#[test]
fn test_validate_system_fail_version_null() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":null}"#).unwrap();
    let err = validate_system(&v).unwrap_err();
    assert!(err.contains("version must be a string"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// validate_system — cron
// ---------------------------------------------------------------------------

#[test]
fn test_validate_system_pass_cron_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"cron":{"enabled":true}}"#).unwrap();
    assert!(validate_system(&v).is_ok());
}

#[test]
fn test_validate_system_pass_cron_absent() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_system(&v).is_ok());
}

#[test]
fn test_validate_system_fail_cron_not_object_variants() {
    let cases = [r#""bad""#, r#"[1,2]"#, r#"true"#];
    for json in cases {
        let v: serde_json::Value =
            serde_json::from_str(&format!(r#"{{"cron":{}}}"#, json)).unwrap();
        let err = validate_system(&v).unwrap_err();
        assert!(
            err.contains("cron must be a JSON object"),
            "error for {}: {}",
            json,
            err
        );
    }
}

#[test]
fn test_validate_system_pass_version_and_cron() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"version":"2.0","cron":{"enabled":false}}"#).unwrap();
    assert!(validate_system(&v).is_ok());
}

// ---------------------------------------------------------------------------
// ConfigSection::default_validator / for_section
// ---------------------------------------------------------------------------

#[test]
fn test_default_validator_models_passes_valid_json() {
    let v: serde_json::Value = serde_json::from_str(r#"{"models":[{"id":"m1"}]}"#).unwrap();
    let validator = ConfigSection::Models.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_models_rejects_non_object() {
    let v: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let validator = ConfigSection::Models.default_validator();
    assert!(validator(&v).is_err());
}

#[test]
fn test_default_validator_channels_passes_valid_json() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"channels":{"feishu":{"enabled":true}}}"#).unwrap();
    let validator = ConfigSection::Channels.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_gateway_passes_valid_json() {
    let v: serde_json::Value = serde_json::from_str(r#"{"port":9090}"#).unwrap();
    let validator = ConfigSection::Gateway.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_plugins_passes_valid_json() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"entries":{"p":{"enabled":true}}}"#).unwrap();
    let validator = ConfigSection::Plugins.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_system_passes_valid_json() {
    let v: serde_json::Value = serde_json::from_str(r#"{"version":"2.0"}"#).unwrap();
    let validator = ConfigSection::System.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_credentials_always_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"null"#).unwrap();
    let validator = ConfigSection::Credentials.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_for_section_returns_correct_validator() {
    let sections = [
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ];
    let valid_json: serde_json::Value = serde_json::from_str(r#"{"a":1}"#).unwrap();
    let invalid_json: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();

    for section in sections {
        let validator = for_section(section);
        // valid object should pass all section validators
        assert!(
            validator(&valid_json).is_ok(),
            "validator for {:?} should pass valid JSON",
            section
        );
        // array should fail all section validators
        assert!(
            validator(&invalid_json).is_err(),
            "validator for {:?} should reject array",
            section
        );
    }
}

#[test]
fn test_for_section_credentials_always_passes() {
    let v: serde_json::Value = serde_json::from_str(r#""anything""#).unwrap();
    let validator = for_section(ConfigSection::Credentials);
    assert!(validator(&v).is_ok());
}

// ---------------------------------------------------------------------------
// Boundary / edge-case tests (Step 1.8)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_pass_empty_providers_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"providers":{}}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_pass_providers_with_empty_models_array() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[]}}}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_plugins_pass_empty_entries_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"entries":{}}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_pass_empty_allow_array() {
    let v: serde_json::Value = serde_json::from_str(r#"{"allow":[]}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

#[test]
fn test_validate_plugins_pass_empty_installs_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"installs":{}}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

// ---------------------------------------------------------------------------
// validate_session
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"sweeperIntervalSecs":600,"compact":{}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_empty_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_no_sweeper_interval() {
    // sweeperIntervalSecs absent — should pass (optional field)
    let v: serde_json::Value = serde_json::from_str(r#"{"compact":{}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_fail_not_object() {
    let v: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_string() {
    let v: serde_json::Value = serde_json::from_str(r#""hello""#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("JSON object"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_sweeper_interval_zero() {
    let v: serde_json::Value = serde_json::from_str(r#"{"sweeperIntervalSecs":0}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("positive number"),
        "error should mention positive number: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_sweeper_interval_string() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"sweeperIntervalSecs":"not a number"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("positive number"),
        "error should mention positive number: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_sweeper_interval_negative() {
    // Negative number: as_u64() returns None for negative → unwrap_or(0) == 0
    let v: serde_json::Value = serde_json::from_str(r#"{"sweeperIntervalSecs":-5}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("positive number"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_sweeper_interval_null() {
    let v: serde_json::Value = serde_json::from_str(r#"{"sweeperIntervalSecs":null}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("positive number"), "error: {}", err);
}

#[test]
fn test_validate_session_pass_sweeper_interval_positive() {
    let v: serde_json::Value = serde_json::from_str(r#"{"sweeperIntervalSecs":1}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// validate_session — idleMinutes
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_idle_minutes_negative() {
    let v: serde_json::Value = serde_json::from_str(r#"{"idleMinutes":-1}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("idleMinutes must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_idle_minutes_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"idleMinutes":"abc"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("idleMinutes must be a number"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_purge_after_minutes_negative() {
    let v: serde_json::Value = serde_json::from_str(r#"{"purgeAfterMinutes":-1}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("purgeAfterMinutes must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_purge_after_minutes_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"purgeAfterMinutes":"abc"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("purgeAfterMinutes must be a number"),
        "error: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// validate_session — combined fields
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_all_session_fields() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"sweeperIntervalSecs":600,"idleMinutes":30,"purgeAfterMinutes":1440,"compact":{}}"#,
    )
    .unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_fail_multiple_invalid() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"idleMinutes":30,"purgeAfterMinutes":-1}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("purgeAfterMinutes must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_default_validator_session_passes_valid_json() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"sweeperIntervalSecs":300,"compact":{}}"#).unwrap();
    let validator = ConfigSection::Session.default_validator();
    assert!(validator(&v).is_ok());
}

#[test]
fn test_default_validator_session_rejects_non_object() {
    let v: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let validator = ConfigSection::Session.default_validator();
    assert!(validator(&v).is_err());
}

#[test]
fn test_for_section_session_returns_validator() {
    let validator = for_section(ConfigSection::Session);
    let valid: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    let invalid: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    assert!(validator(&valid).is_ok());
    assert!(validator(&invalid).is_err());
}

// ---------------------------------------------------------------------------
// Step 1.4 — channels binding channel reference validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_channels_binding_ref_valid_channel_type() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":"acc1"}}]}"#,
    )
    .unwrap();
    assert!(validate_channels(&v).is_ok());
}

#[test]
fn test_validate_channels_binding_ref_undefined_channel_type() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"slack","accountId":"acc1"}}]}"#,
    )
    .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(
        err.contains("references an undefined channel type"),
        "error: {}",
        err
    );
    assert!(
        err.contains("slack"),
        "error should mention the bad channel: {}",
        err
    );
}

#[test]
fn test_validate_channels_binding_ref_no_channels_with_bindings() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":"acc1"}}]}"#,
    )
    .unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(
        err.contains("references an undefined channel type"),
        "error: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// Step 1.4 — models credentialPath file existence validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_credential_path_exists() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let json = format!(
        r#"{{"providers":{{"p":{{"credentialPath":"{}","models":[]}}}}}}"#,
        path
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_credential_path_not_exists() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"providers":{"p":{"credentialPath":"/nonexistent/path/to/cred","models":[]}}}"#,
    )
    .unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("does not exist"), "error: {}", err);
    assert!(
        err.contains("credentialPath"),
        "error should mention credentialPath: {}",
        err
    );
}

#[test]
fn test_validate_models_credential_path_null() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"credentialPath":null,"models":[]}}}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_credential_path_absent() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[]}}}"#).unwrap();
    assert!(validate_models(&v).is_ok());
}
