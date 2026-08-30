//! Step 1.9 — validate_session planArchive / auditLog validation tests.

use crate::validators::validate_session;

// ---------------------------------------------------------------------------
// planArchive — valid cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_plan_archive_valid_object() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"planArchive":{"thresholdDays":30}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_plan_archive_empty_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"planArchive":{}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_plan_archive_absent() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_plan_archive_zero_threshold_days() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"planArchive":{"thresholdDays":0}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// planArchive — error cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_plan_archive_not_object() {
    let cases = [
        (
            r#"{"planArchive":"bad"}"#,
            "planArchive must be a JSON object, got string",
        ),
        (
            r#"{"planArchive":[1,2]}"#,
            "planArchive must be a JSON object, got array",
        ),
        (
            r#"{"planArchive":true}"#,
            "planArchive must be a JSON object, got boolean",
        ),
        (
            r#"{"planArchive":null}"#,
            "planArchive must be a JSON object, got null",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

#[test]
fn test_validate_session_fail_plan_archive_threshold_days_negative() {
    let cases = [
        (
            r#"{"planArchive":{"thresholdDays":-1}}"#,
            "thresholdDays must be non-negative",
        ),
        (
            r#"{"planArchive":{"thresholdDays":"abc"}}"#,
            "thresholdDays must be a number",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

// ---------------------------------------------------------------------------
// auditLog — valid cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_audit_log_valid_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"auditLog":{"maxEntries":1000}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_audit_log_empty_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{"auditLog":{}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_audit_log_absent() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_audit_log_zero_max_entries() {
    let v: serde_json::Value = serde_json::from_str(r#"{"auditLog":{"maxEntries":0}}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// auditLog — error cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_audit_log_not_object() {
    let cases = [
        (
            r#"{"auditLog":"bad"}"#,
            "auditLog must be a JSON object, got string",
        ),
        (
            r#"{"auditLog":[1,2]}"#,
            "auditLog must be a JSON object, got array",
        ),
        (
            r#"{"auditLog":true}"#,
            "auditLog must be a JSON object, got boolean",
        ),
        (
            r#"{"auditLog":null}"#,
            "auditLog must be a JSON object, got null",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

#[test]
fn test_validate_session_fail_audit_log_max_entries_negative() {
    let cases = [
        (
            r#"{"auditLog":{"maxEntries":-1}}"#,
            "maxEntries must be non-negative",
        ),
        (
            r#"{"auditLog":{"maxEntries":"abc"}}"#,
            "maxEntries must be a number",
        ),
    ];
    for (json, expected) in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_session(&v).unwrap_err();
        assert!(err.contains(expected), "json={}: error: {}", json, err);
    }
}

// ---------------------------------------------------------------------------
// Combined fields
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_both_plan_archive_and_audit_log() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"planArchive":{"thresholdDays":30},"auditLog":{"maxEntries":500}}"#,
    )
    .unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_fail_plan_archive_invalid_with_valid_audit_log() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"planArchive":"bad","auditLog":{"maxEntries":500}}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("planArchive must be a JSON object"),
        "error: {}",
        err
    );
}
