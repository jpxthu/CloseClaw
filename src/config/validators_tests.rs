//! Tests for config section validators.

use crate::config::manager::ConfigSection;
use crate::config::validators::{
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
    let v: serde_json::Value = serde_json::from_str(r#"{"models":[{"id":"m1"}]}"#).unwrap();
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

// ---------------------------------------------------------------------------
// validate_channels
// ---------------------------------------------------------------------------

#[test]
fn test_validate_channels_pass() {
    let v: serde_json::Value = serde_json::from_str(r#"{"channels":[{"id":"ch1"}]}"#).unwrap();
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
    assert!(err.contains("array"), "error: {}", err);
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

// ---------------------------------------------------------------------------
// validate_plugins
// ---------------------------------------------------------------------------

#[test]
fn test_validate_plugins_pass() {
    let v: serde_json::Value = serde_json::from_str(r#"{"plugins":[{"id":"p1"}]}"#).unwrap();
    assert!(validate_plugins(&v).is_ok());
}

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
fn test_validate_plugins_fail_plugins_not_array() {
    let v: serde_json::Value = serde_json::from_str(r#"{"plugins":42}"#).unwrap();
    let err = validate_plugins(&v).unwrap_err();
    assert!(err.contains("array"), "error: {}", err);
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
    let v: serde_json::Value = serde_json::from_str(r#"{"channels":[{"id":"ch1"}]}"#).unwrap();
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
    let v: serde_json::Value = serde_json::from_str(r#"{"plugins":[{"id":"p1"}]}"#).unwrap();
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
