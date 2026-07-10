//! Tests for config section validators.

use std::collections::HashSet;

use crate::manager::ConfigSection;
use crate::validators::{
    for_section, validate_channels, validate_channels_with_refs, validate_gateway, validate_models,
    validate_models_with_refs, validate_plugins, validate_session, validate_system,
    CredentialProviderSet, CrossRefData,
};

// ---------------------------------------------------------------------------
// validate_models
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_pass_empty_object_or_with_array() {
    for json in [r#"{}"#, r#"{"models":[{"id":"m1"}]}"#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_models(&v).is_ok(), "json={}", json);
    }
}

#[test]
fn test_validate_models_fail_not_object_variants() {
    for json in [r#""string""#, r#"[1,2,3]"#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_models(&v).unwrap_err();
        assert!(err.contains("JSON object"), "json={}: error: {}", json, err);
    }
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
fn test_validate_models_pass_valid_or_empty_base_url() {
    for url in ["https://api.example.com", ""] {
        let json = format!(
            r#"{{"providers":{{"p":{{"baseUrl":"{}","models":[]}}}}}}"#,
            url
        );
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(validate_models(&v).is_ok(), "url={}", url);
    }
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
fn test_validate_channels_pass_variants() {
    for json in [
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":"acc1"}}]}"#,
        r#"{}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_channels(&v).is_ok(), "json={}", json);
    }
}

#[test]
fn test_validate_channels_fail_not_object_or_bad_type() {
    for json in [r#"[1]"#, r#"{"channels":"bad"}"#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_channels(&v).unwrap_err();
        assert!(err.contains("JSON object"), "json={}: error: {}", json, err);
    }
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
fn test_validate_channels_fail_binding_agent_id() {
    let cases = [
        (
            r#"{"bindings":[{"match":{"channel":"feishu","accountId":"a"}}]}"#,
            "agentId is required",
        ),
        (
            r#"{"bindings":[{"agentId":"","match":{"channel":"feishu","accountId":"a"}}]}"#,
            "agentId cannot be empty",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_channels(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

#[test]
fn test_validate_channels_fail_binding_match() {
    let cases = [
        (r#"{"bindings":[{"agentId":"a1"}]}"#, "match is required"),
        (
            r#"{"bindings":[{"agentId":"a1","match":"bad"}]}"#,
            "match must be a JSON object",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_channels(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

#[test]
fn test_validate_channels_fail_binding_channel() {
    let cases = [
        (
            r#"{"bindings":[{"agentId":"a1","match":{"accountId":"a"}}]}"#,
            "match.channel is required",
        ),
        (
            r#"{"bindings":[{"agentId":"a1","match":{"channel":"","accountId":"a"}}]}"#,
            "match.channel cannot be empty",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_channels(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

#[test]
fn test_validate_channels_fail_binding_account_id() {
    let cases = [
        (
            r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu"}}]}"#,
            "match.accountId is required",
        ),
        (
            r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":""}}]}"#,
            "match.accountId cannot be empty",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_channels(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
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
    let cases = [
        (r#"{"port":0}"#, "range 1-65535"),
        (r#"{"port":99999}"#, "range 1-65535"),
        (r#"{"port":"abc"}"#, "non-negative integer"),
        (r#"{"port":-1}"#, "non-negative integer"),
        (r#"{"timeout":-1000}"#, "non-negative"),
        (r#"{"timeout":"not-a-number"}"#, "must be a number"),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_gateway(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

#[test]
fn test_validate_gateway_pass_valid_values() {
    for json in [
        r#"{"timeout":30000}"#,
        r#"{"timeout":0}"#,
        r#"{"port":8080,"timeout":30000,"name":"gw"}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_gateway(&v).is_ok(), "json={}", json);
    }
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
fn test_validate_system_version() {
    // valid cases
    for json in [r#"{"version":"1.0"}"#, r#"{}"#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_system(&v).is_ok(), "json={}", json);
    }
    // invalid cases
    let cases = [
        (r#"{"version":""}"#, "version cannot be an empty string"),
        (r#"{"version":123}"#, "version must be a string"),
        (r#"{"version":null}"#, "version must be a string"),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_system(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
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
// CrossRefData — channels binding cross-reference validation
// ---------------------------------------------------------------------------

#[test]
fn test_validate_channels_cross_ref_unknown_agent_id() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"unknown-agent","match":{"channel":"feishu","accountId":"acc1"}}]}"#,
    )
    .unwrap();
    let cr = CrossRefData {
        agent_ids: ["known-agent".to_string()].into_iter().collect(),
        account_ids: ["acc1".to_string()].into_iter().collect(),
    };
    let err = validate_channels_with_refs(&v, Some(&cr)).unwrap_err();
    assert!(
        err.contains("references an unknown agent"),
        "error: {}",
        err
    );
    assert!(
        err.contains("unknown-agent"),
        "error should mention the bad agent: {}",
        err
    );
}

#[test]
fn test_validate_channels_cross_ref_unknown_account_id() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"known-agent","match":{"channel":"feishu","accountId":"unknown-account"}}]}"#,
    )
    .unwrap();
    let cr = CrossRefData {
        agent_ids: ["known-agent".to_string()].into_iter().collect(),
        account_ids: ["known-account".to_string()].into_iter().collect(),
    };
    let err = validate_channels_with_refs(&v, Some(&cr)).unwrap_err();
    assert!(
        err.contains("references an unknown account"),
        "error: {}",
        err
    );
    assert!(
        err.contains("unknown-account"),
        "error should mention the bad account: {}",
        err
    );
}

#[test]
fn test_validate_channels_cross_ref_known_agent_and_account() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"agent-1","match":{"channel":"feishu","accountId":"acc-1"}}]}"#,
    )
    .unwrap();
    let cr = CrossRefData {
        agent_ids: ["agent-1".to_string(), "agent-2".to_string()]
            .into_iter()
            .collect(),
        account_ids: ["acc-1".to_string(), "acc-2".to_string()]
            .into_iter()
            .collect(),
    };
    assert!(validate_channels_with_refs(&v, Some(&cr)).is_ok());
}

#[test]
fn test_validate_channels_cross_ref_none_skips_validation() {
    // With None cross-ref, binding structural validation only — agentId/accountId not checked
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"any-agent","match":{"channel":"feishu","accountId":"any-account"}}]}"#,
    )
    .unwrap();
    assert!(validate_channels_with_refs(&v, None).is_ok());
}

// ---------------------------------------------------------------------------
// Boundary / edge-case tests (Step 1.8)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_pass_empty_providers_or_models() {
    let v1: serde_json::Value = serde_json::from_str(r#"{"providers":{}}"#).unwrap();
    assert!(validate_models(&v1).is_ok());
    let v2: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[]}}}"#).unwrap();
    assert!(validate_models(&v2).is_ok());
}

#[test]
fn test_validate_plugins_pass_empty_collections() {
    let v1: serde_json::Value = serde_json::from_str(r#"{"entries":{}}"#).unwrap();
    assert!(validate_plugins(&v1).is_ok());
    let v2: serde_json::Value = serde_json::from_str(r#"{"allow":[]}"#).unwrap();
    assert!(validate_plugins(&v2).is_ok());
    let v3: serde_json::Value = serde_json::from_str(r#"{"installs":{}}"#).unwrap();
    assert!(validate_plugins(&v3).is_ok());
}

// ---------------------------------------------------------------------------
// validate_session
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_variants() {
    for json in [
        r#"{"sweeperIntervalSecs":600,"compact":{}}"#,
        r#"{}"#,
        r#"{"compact":{}}"#,
        r#"{"sweeperIntervalSecs":1}"#,
        r#"{"sweeperIntervalSecs":600,"idleMinutes":30,"purgeAfterMinutes":1440,"compact":{}}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_session(&v).is_ok(), "json={}", json);
    }
}

#[test]
fn test_validate_session_fail_invalid_type_and_sweeper() {
    for json in [r#"[1]"#, r#""hello""#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains("JSON object"), "{}: error: {}", json, err);
    }
    let cases = [
        r#"{"sweeperIntervalSecs":0}"#,
        r#"{"sweeperIntervalSecs":"not a number"}"#,
        r#"{"sweeperIntervalSecs":-5}"#,
        r#"{"sweeperIntervalSecs":null}"#,
    ];
    for json in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains("positive number"), "{}: {}", json, err);
    }
}

// ---------------------------------------------------------------------------
// validate_session — idleMinutes
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_idle_purge_invalid() {
    let cases = [
        (r#"{"idleMinutes":-1}"#, "idleMinutes must be non-negative"),
        (r#"{"idleMinutes":"abc"}"#, "idleMinutes must be a number"),
        (
            r#"{"purgeAfterMinutes":-1}"#,
            "purgeAfterMinutes must be non-negative",
        ),
        (
            r#"{"purgeAfterMinutes":"abc"}"#,
            "purgeAfterMinutes must be a number",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains(expected), "{}: error: {}", json, err);
    }
}

// ---------------------------------------------------------------------------
// validate_session — combined fields
// ---------------------------------------------------------------------------

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
fn test_default_validator_session_and_for_section() {
    let valid: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    let invalid: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    assert!(ConfigSection::Session.default_validator()(&valid).is_ok());
    assert!(ConfigSection::Session.default_validator()(&invalid).is_err());
    let validator = for_section(ConfigSection::Session);
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
fn test_validate_models_credential_path_null_or_absent() {
    let v_null: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"credentialPath":null,"models":[]}}}"#).unwrap();
    assert!(validate_models(&v_null).is_ok());
    let v_absent: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[]}}}"#).unwrap();
    assert!(validate_models(&v_absent).is_ok());
}

// ---------------------------------------------------------------------------
// Step 1.3 — models apiKey credential cross-validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_api_key_ref_unknown_cred_provider() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"apiKey":"sk-test","models":[]}}}"#)
            .unwrap();
    let crps = CredentialProviderSet {
        names: ["anthropic".to_string()].into_iter().collect(),
    };
    let err = validate_models_with_refs(&v, Some(&crps)).unwrap_err();
    assert!(
        err.contains("references an unknown credential provider"),
        "error: {}",
        err
    );
    assert!(
        err.contains("openai"),
        "error should mention openai: {}",
        err
    );
}

#[test]
fn test_validate_models_api_key_ref_known_cred_provider() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"apiKey":"sk-test","models":[]}}}"#)
            .unwrap();
    let crps = CredentialProviderSet {
        names: ["openai".to_string(), "anthropic".to_string()]
            .into_iter()
            .collect(),
    };
    assert!(validate_models_with_refs(&v, Some(&crps)).is_ok());
}

#[test]
fn test_validate_models_no_api_key_passes() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"models":[{"id":"gpt-4"}]}}}"#).unwrap();
    let crps = CredentialProviderSet {
        names: HashSet::new(),
    };
    assert!(validate_models_with_refs(&v, Some(&crps)).is_ok());
}

#[test]
fn test_validate_models_api_key_with_credential_path_passes() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let json = format!(
        r#"{{"providers":{{"openai":{{"apiKey":"sk-test","credentialPath":"{}","models":[]}}}}}}"#,
        path
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let crps = CredentialProviderSet {
        names: HashSet::new(),
    };
    assert!(validate_models_with_refs(&v, Some(&crps)).is_ok());
}

#[test]
fn test_validate_models_no_cross_ref_skips_check() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"apiKey":"sk-test","models":[]}}}"#)
            .unwrap();
    assert!(validate_models_with_refs(&v, None).is_ok());
}
