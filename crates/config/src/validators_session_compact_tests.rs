//! Step 1.2 — validate_session compact field validation tests.
//!
//! Covers:
//! - Normal: valid compact, absent compact, null compact
//! - Error: chars_per_token <= 0, thresholds out of [0,1], warning <= auto, non-object
//! - Boundary: thresholds at 0 and 1 exactly

use crate::validators::validate_session;

// ---------------------------------------------------------------------------
// Normal — valid compact
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_compact_valid() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.05,"warningThresholdPct":0.10,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// Normal — compact absent
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_compact_absent() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// Normal — compact is null
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_compact_null() {
    let v: serde_json::Value = serde_json::from_str(r#"{"compact":null}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// Error — chars_per_token <= 0
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_compact_chars_per_token_zero() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.0,"autoCompactThresholdPct":0.05,"warningThresholdPct":0.10,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
    assert!(err.contains("chars_per_token"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_compact_chars_per_token_negative() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":-1.0,"autoCompactThresholdPct":0.05,"warningThresholdPct":0.10,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
    assert!(err.contains("chars_per_token"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// Error — thresholds out of [0, 1]
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_compact_auto_threshold_negative() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":-0.1,"warningThresholdPct":0.10,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
    assert!(err.contains("auto_compact_threshold_pct"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_compact_auto_threshold_over_one() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":1.5,"warningThresholdPct":2.0,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_compact_warning_threshold_negative() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.05,"warningThresholdPct":-0.1,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
    assert!(err.contains("warning_threshold_pct"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_compact_warning_threshold_over_one() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.05,"warningThresholdPct":1.5,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
    assert!(err.contains("warning_threshold_pct"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// Error — warning <= auto (including equality boundary)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_compact_warning_equals_auto() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.05,"warningThresholdPct":0.05,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
    assert!(err.contains("auto_compact_threshold_pct"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_compact_warning_less_than_auto() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.10,"warningThresholdPct":0.05,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
    assert!(err.contains("auto_compact_threshold_pct"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// Error — compact is not an object
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_compact_string() {
    let v: serde_json::Value = serde_json::from_str(r#"{"compact":"invalid"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
}

#[test]
fn test_validate_session_fail_compact_number() {
    let v: serde_json::Value = serde_json::from_str(r#"{"compact":42}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
}

// ---------------------------------------------------------------------------
// Boundary — thresholds at exactly 0 and 1
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_compact_thresholds_at_zero() {
    // auto at exactly 0 boundary with warning > auto
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.0,"warningThresholdPct":0.1,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_compact_thresholds_at_one() {
    // warning at exactly 1 boundary with auto < warning
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.9,"warningThresholdPct":1.0,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_compact_auto_at_zero_warning_at_one() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"charsPerToken":0.25,"autoCompactThresholdPct":0.0,"warningThresholdPct":1.0,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// Error — missing required fields (deserialization failure)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_compact_missing_chars_per_token() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"compact":{"autoCompactThresholdPct":0.05,"warningThresholdPct":0.10,"maxConsecutiveFailures":3}}"#,
    )
    .unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(err.contains("session.compact"), "error: {}", err);
}
