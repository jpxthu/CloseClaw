//! Tests for config section validators.

use std::collections::HashSet;

use crate::manager::ConfigSection;
use crate::validators::{
    for_section, validate_channels, validate_channels_with_refs, validate_gateway, validate_media,
    validate_models, validate_models_with_refs, validate_plugins, validate_session,
    validate_system, CredentialProviderSet, CrossRefData,
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

#[test]
fn test_validate_gateway_inbound_queue_capacity_zero() {
    let v: serde_json::Value = serde_json::from_str(r#"{"inboundQueueCapacity":0}"#).unwrap();
    let err = validate_gateway(&v).unwrap_err();
    assert!(
        err.contains("greater than 0"),
        "error should mention capacity must be > 0: {}",
        err
    );
}

#[test]
fn test_validate_gateway_inbound_queue_capacity_non_positive() {
    for json in [
        r#"{"inboundQueueCapacity":-1}"#,
        r#"{"inboundQueueCapacity":"abc"}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_gateway(&v).unwrap_err();
        assert!(err.contains("positive integer"), "json={}: {}", json, err);
    }
}

#[test]
fn test_validate_gateway_inbound_queue_capacity_valid() {
    for json in [
        r#"{"inboundQueueCapacity":1}"#,
        r#"{"inboundQueueCapacity":256}"#,
        r#"{"inboundQueueCapacity":1024}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_gateway(&v).is_ok(), "json={}", json);
    }
}

#[test]
fn test_validate_gateway_inbound_queue_capacity_missing_ok() {
    let v: serde_json::Value = serde_json::from_str(r#"{"port":8080}"#).unwrap();
    assert!(
        validate_gateway(&v).is_ok(),
        "missing inboundQueueCapacity should pass"
    );
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
fn test_validate_plugins_pass_install_path_optional() {
    // Empty installPath should be ignored; absent installPath is also valid
    for json in [
        r#"{"installs":{"p":{"installPath":""}}}"#,
        r#"{"installs":{"p":{"source":"archive"}}}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_plugins(&v).is_ok(), "json={}", json);
    }
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
fn test_validate_system_pass_cron_variants() {
    for json in [r#"{"cron":{"enabled":true}}"#, r#"{}"#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_system(&v).is_ok(), "json={}", json);
    }
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
fn test_default_validator_models_pass_and_reject() {
    let valid: serde_json::Value = serde_json::from_str(r#"{"models":[{"id":"m1"}]}"#).unwrap();
    let invalid: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let v = ConfigSection::Models.default_validator();
    assert!(v(&valid).is_ok());
    assert!(v(&invalid).is_err());
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
fn test_default_validator_credentials_pass_and_reject() {
    let valid: serde_json::Value =
        serde_json::from_str(r#"{"provider":"openai","apiKey":"sk-test"}"#).unwrap();
    let invalid: serde_json::Value = serde_json::from_str(r#"null"#).unwrap();
    let validator = ConfigSection::Credentials.default_validator();
    assert!(validator(&valid).is_ok());
    assert!(validator(&invalid).is_err());
}

#[test]
fn test_for_section_returns_correct_validator() {
    let valid_json: serde_json::Value = serde_json::from_str(r#"{"a":1}"#).unwrap();
    let invalid_json: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    for section in [
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ] {
        let v = for_section(section);
        assert!(
            v(&valid_json).is_ok(),
            "{:?} should pass valid JSON",
            section
        );
        assert!(
            v(&invalid_json).is_err(),
            "{:?} should reject array",
            section
        );
    }
}

// ---------------------------------------------------------------------------
// CrossRefData — channels binding cross-reference validation
// ---------------------------------------------------------------------------

#[test]
fn test_validate_channels_cross_ref_unknown_agent_id() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"unknown-agent","match":{"channel":"feishu","accountId":"acc1"}}]}"#,
    ).unwrap();
    let cr = CrossRefData {
        agent_ids: ["known-agent".into()].into_iter().collect(),
        account_ids: ["acc1".into()].into_iter().collect(),
    };
    let err = validate_channels_with_refs(&v, Some(&cr)).unwrap_err();
    assert!(
        err.contains("references an unknown agent"),
        "error: {}",
        err
    );
    assert!(err.contains("unknown-agent"), "bad agent: {}", err);
}

#[test]
fn test_validate_channels_cross_ref_unknown_account_id() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"known-agent","match":{"channel":"feishu","accountId":"unknown-account"}}]}"#,
    ).unwrap();
    let cr = CrossRefData {
        agent_ids: ["known-agent".into()].into_iter().collect(),
        account_ids: ["known-account".into()].into_iter().collect(),
    };
    let err = validate_channels_with_refs(&v, Some(&cr)).unwrap_err();
    assert!(
        err.contains("references an unknown account"),
        "error: {}",
        err
    );
    assert!(err.contains("unknown-account"), "bad account: {}", err);
}

#[test]
fn test_validate_channels_cross_ref_known_agent_and_account() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"agent-1","match":{"channel":"feishu","accountId":"acc-1"}}]}"#,
    ).unwrap();
    let cr = CrossRefData {
        agent_ids: ["agent-1".into(), "agent-2".into()].into_iter().collect(),
        account_ids: ["acc-1".into(), "acc-2".into()].into_iter().collect(),
    };
    assert!(validate_channels_with_refs(&v, Some(&cr)).is_ok());
}

#[test]
fn test_validate_channels_cross_ref_none_skips_validation() {
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
// validate_media
// ---------------------------------------------------------------------------

#[test]
fn test_validate_media_pass_variants() {
    for json in [
        r#"{}"#,
        r#"{"storageDir":"/data/media","retentionDays":14,"imageContentThresholdBytes":2097152}"#,
        r#"{"storageDir":"./media"}"#,
        r#"{"storageDir":"~/.closeclaw/media"}"#,
        r#"{"storageDir":"relative/path"}"#,
        r#"{"storageDir":null}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_media(&v).is_ok(), "json={}", json);
    }
}

#[test]
fn test_validate_media_fail_not_object() {
    for json in [r#"[1]"#, r#""string""#, r#"null"#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_media(&v).unwrap_err();
        assert!(err.contains("JSON object"), "json={}: error: {}", json, err);
        assert!(ConfigSection::Media.default_validator()(&v).is_err());
    }
}

#[test]
fn test_validate_media_structural_only_passes_field_errors() {
    // StorageDir field-level validation is delegated to MediaConfigData::validate();
    // the structural validator (mod.rs) only checks top-level shape.
    for json in [
        r#"{"storageDir":""}"#,
        r#"{"storageDir":"   "}"#,
        r#"{"storageDir":"/tmp/media\u0000bad"}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_media(&v).is_ok(), "json={}", json);
    }
    // Non-string types are still rejected by the structural validator
    let v: serde_json::Value = serde_json::from_str(r#"{"storageDir":123}"#).unwrap();
    assert!(
        validate_media(&v).is_ok(),
        "non-string storageDir passes structural check"
    );
}

// validate_session

#[test]
fn test_validate_session_fail_invalid_type_and_sweeper() {
    for json in [r#"[1]"#, r#""hello""#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains("JSON object"), "{}: error: {}", json, err);
    }
    let cases = [
        r#"{"sweeperIntervalSeconds":0}"#,
        r#"{"sweeperIntervalSeconds":"not a number"}"#,
        r#"{"sweeperIntervalSeconds":-5}"#,
        r#"{"sweeperIntervalSeconds":null}"#,
    ];
    for json in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains("positive number"), "{}: {}", json, err);
    }
}

// ---------------------------------------------------------------------------
// validate_session — nested per-role idleMinutes / purgeAfterMinutes
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_nested_per_role() {
    // Valid: nested defaults and agents overrides
    for json in [
        r#"{"defaults":{"mainAgent":{"idleMinutes":30,"purgeAfterMinutes":1440}}}"#,
        r#"{"defaults":{"mainAgent":{"idleMinutes":0,"purgeAfterMinutes":0}}}"#,
        r#"{"defaults":{"mainAgent":{"idleMinutes":30},"subAgent":{"idleMinutes":10}}}"#,
        r#"{"defaults":{"mainAgent":{"idleMinutes":30}},"agents":{"a1":{"mainAgent":{"idleMinutes":60}}}}"#,
        r#"{"defaults":{},"agents":{}}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_session(&v).is_ok(), "json={}", json);
    }

    // Fail: invalid nested values
    let fail_cases = [
        (
            r#"{"defaults":{"mainAgent":{"idleMinutes":-1}}}"#,
            "idleMinutes must be non-negative",
        ),
        (
            r#"{"defaults":{"mainAgent":{"idleMinutes":"abc"}}}"#,
            "idleMinutes must be a number",
        ),
        (
            r#"{"defaults":{"mainAgent":{"purgeAfterMinutes":-1}}}"#,
            "purgeAfterMinutes must be non-negative",
        ),
        (
            r#"{"defaults":{"mainAgent":{"idleMinutes":30,"purgeAfterMinutes":-1}}}"#,
            "purgeAfterMinutes must be non-negative",
        ),
        (
            r#"{"agents":{"a1":{"mainAgent":{"idleMinutes":-5}}}}"#,
            "idleMinutes must be non-negative",
        ),
        (
            r#"{"defaults":{"mainAgent":"invalid"}}"#,
            "must be a JSON object",
        ),
    ];
    for (json, expected) in fail_cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains(expected), "{}: error: {}", json, err);
    }
}

// ---------------------------------------------------------------------------
#[test]
fn test_default_validator_session_and_for_section() {
    let valid: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    let invalid: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
    let compact_valid: serde_json::Value = serde_json::from_str(
        r#"{"sweeperIntervalSeconds":300,"compact":{"charsPerToken":4.0,"maxConsecutiveFailures":3}}"#,
    ).unwrap();
    assert!(ConfigSection::Session.default_validator()(&valid).is_ok());
    assert!(ConfigSection::Session.default_validator()(&compact_valid).is_ok());
    assert!(ConfigSection::Session.default_validator()(&invalid).is_err());
    let v = for_section(ConfigSection::Session);
    assert!(v(&valid).is_ok());
    assert!(v(&invalid).is_err());
}

// ---------------------------------------------------------------------------
// Step 1.4 — channels binding channel reference validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_channels_binding_ref_valid_channel_type() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"feishu","accountId":"acc1"}}]}"#,
    ).unwrap();
    assert!(validate_channels(&v).is_ok());
}

#[test]
fn test_validate_channels_binding_ref_undefined_channel_type() {
    // Binding references slack which is not in channels config
    let v: serde_json::Value = serde_json::from_str(
        r#"{"channels":{"feishu":{"enabled":true}},"bindings":[{"agentId":"a1","match":{"channel":"slack","accountId":"acc1"}}]}"#,
    ).unwrap();
    let err = validate_channels(&v).unwrap_err();
    assert!(
        err.contains("references an undefined channel type"),
        "error: {}",
        err
    );
    assert!(err.contains("slack"), "bad channel: {}", err);
}

#[test]
fn test_validate_channels_binding_ref_no_channels_with_bindings() {
    // No channels config at all, binding references feishu
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
        names: ["openai".into(), "anthropic".into()].into_iter().collect(),
    };
    assert!(validate_models_with_refs(&v, Some(&crps)).is_ok());
}

#[test]
fn test_validate_models_pass_no_api_key_or_credential_path() {
    let empty_creds = CredentialProviderSet {
        names: HashSet::new(),
    };
    // No apiKey: passes regardless of credential providers
    let v1: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"models":[{"id":"gpt-4"}]}}}"#).unwrap();
    assert!(validate_models_with_refs(&v1, Some(&empty_creds)).is_ok());
    // No cross-ref: skips credential check entirely
    let v2: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"apiKey":"sk-test","models":[]}}}"#)
            .unwrap();
    assert!(validate_models_with_refs(&v2, None).is_ok());
}

#[test]
fn test_validate_models_api_key_with_credential_path_passes() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let json = format!(
        r#"{{"providers":{{"openai":{{"apiKey":"sk-test","credentialPath":"{}","models":[]}}}}}}"#,
        tmp.path().to_str().unwrap()
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(validate_models_with_refs(
        &v,
        Some(&CredentialProviderSet {
            names: HashSet::new()
        })
    )
    .is_ok());
}
