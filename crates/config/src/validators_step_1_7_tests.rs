//! Step 1.7 — E2 Review B fixes tests.
//!
//! Covers:
//! - storageDir: null semantics consistent across mod.rs and provider paths
//! - defaults/agents non-object → rejected (session)
//! - is_finite() defense in session non-negative checks (via existing valid/invalid tests)
//! - Memory dedup: all non-negative validators converge

use crate::providers::media::MediaConfigData;
use crate::providers::ConfigProvider;
use crate::validators::memory::validate_memory;
use crate::validators::{validate_media, validate_session};

// ---------------------------------------------------------------------------
// 1. storageDir: null — both paths agree
// ---------------------------------------------------------------------------

#[test]
fn test_media_storage_dir_null_structural_validator_passes() {
    // mod.rs structural validator should accept null (delegates to provider)
    let v: serde_json::Value = serde_json::from_str(r#"{"storageDir":null}"#).unwrap();
    assert!(validate_media(&v).is_ok());
}

#[test]
fn test_media_storage_dir_null_provider_default_is_valid() {
    // Provider deserializes null → default via custom deserializer; validate() passes
    let config = MediaConfigData::from_json_str(r#"{"storageDir":null}"#)
        .expect("null storageDir should deserialize to default");
    assert!(config.validate().is_ok());
    // The actual storage_dir should be the default
    assert_eq!(config.storage_dir, "~/.closeclaw/media");
}

#[test]
fn test_media_storage_dir_absent_provider_default_is_valid() {
    // Absent storageDir → default via #[serde(default)]
    let config = MediaConfigData::from_json_str(r#"{}"#)
        .expect("empty JSON should deserialize with defaults");
    assert!(config.validate().is_ok());
    assert_eq!(config.storage_dir, "~/.closeclaw/media");
}

// ---------------------------------------------------------------------------
// 2. is_finite() defense — session NaN/Infinity
//    Note: serde_json does not parse NaN/Infinity from JSON strings, so
//    these values cannot occur through normal JSON deserialization.
//    The is_finite() check is defense-in-depth for edge cases where
//    values may be constructed programmatically.
//    We verify the defense via the existing non-negative tests and
//    confirm that normal finite values pass.
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_finite_zero_values() {
    // Zero is finite and non-negative — should pass
    let v: serde_json::Value =
        serde_json::from_str(r#"{"defaults":{"main":{"idleMinutes":0,"purgeAfterMinutes":0}}}"#)
            .unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_large_finite_values() {
    // Large but finite values should pass
    let v: serde_json::Value =
        serde_json::from_str(r#"{"planArchiveDays":36500,"auditLogLimit":999999}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// 3. defaults / agents non-object → rejected
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_defaults_null() {
    let v: serde_json::Value = serde_json::from_str(r#"{"defaults":null}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("session.defaults must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_defaults_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"defaults":"bad"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("session.defaults must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_defaults_array() {
    let v: serde_json::Value = serde_json::from_str(r#"{"defaults":[1,2]}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("session.defaults must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_agents_null() {
    let v: serde_json::Value = serde_json::from_str(r#"{"agents":null}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("session.agents must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_agents_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"agents":"bad"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("session.agents must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_agents_array() {
    let v: serde_json::Value = serde_json::from_str(r#"{"agents":[1,2]}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("session.agents must be a JSON object"),
        "error: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// 4. Memory dedup — all non-negative validators converge
// ---------------------------------------------------------------------------

#[test]
fn test_validate_memory_negative_i32_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"mining":{"maxEventsPerSession":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(err.contains("must be non-negative"), "error: {}", err);
}

#[test]
fn test_validate_memory_negative_i64_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"forgetting":{"initialTtlDays":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(err.contains("must be non-negative"), "error: {}", err);
}

#[test]
fn test_validate_memory_negative_u64_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"search":{"timeoutMs":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(err.contains("must be non-negative"), "error: {}", err);
}

#[test]
fn test_validate_memory_negative_u32_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"search":{"minEntityHits":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(err.contains("must be non-negative"), "error: {}", err);
}

#[test]
fn test_validate_memory_negative_usize_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"search":{"contextTurns":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(err.contains("must be non-negative"), "error: {}", err);
}

#[test]
fn test_validate_memory_zero_all_types_pass() {
    let json = r#"{
        "mining": {"maxEventsPerSession": 0, "dedupWindowDays": 0},
        "search": {"timeoutMs": 0, "contextTurns": 0, "maxSummaryChars": 0, "minEntityHits": 0, "topKEvents": 0},
        "forgetting": {"initialTtlDays": 0, "reidentifyExtensionDays": 0, "injectionExtensionDays": 0}
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert!(validate_memory(&v).is_ok());
}

#[test]
fn test_validate_memory_not_number_rejected() {
    // Non-numeric types rejected across all validator variants
    let cases = [
        (
            r#"{"mining":{"maxEventsPerSession":"abc"}}"#,
            "memory.mining.maxEventsPerSession must be a number",
        ),
        (
            r#"{"forgetting":{"initialTtlDays":"abc"}}"#,
            "memory.forgetting.initialTtlDays must be a number",
        ),
        (
            r#"{"search":{"timeoutMs":"abc"}}"#,
            "memory.search.timeoutMs must be a number",
        ),
        (
            r#"{"search":{"minEntityHits":"abc"}}"#,
            "memory.search.minEntityHits must be a number",
        ),
        (
            r#"{"search":{"contextTurns":"abc"}}"#,
            "memory.search.contextTurns must be a number",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_memory(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}
