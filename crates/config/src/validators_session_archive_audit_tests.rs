//! Step 1.9 — validate_session planArchiveDays / auditLogLimit validation tests.

use crate::validators::validate_session;

// ---------------------------------------------------------------------------
// planArchiveDays — valid cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_plan_archive_days_valid() {
    let v: serde_json::Value = serde_json::from_str(r#"{"planArchiveDays":30}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_plan_archive_days_zero() {
    let v: serde_json::Value = serde_json::from_str(r#"{"planArchiveDays":0}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_plan_archive_days_absent() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// planArchiveDays — error cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_plan_archive_days_negative() {
    let v: serde_json::Value = serde_json::from_str(r#"{"planArchiveDays":-1}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("planArchiveDays must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_plan_archive_days_not_number() {
    let v: serde_json::Value = serde_json::from_str(r#"{"planArchiveDays":"abc"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("planArchiveDays must be a number"),
        "error: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// auditLogLimit — valid cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_audit_log_limit_valid() {
    let v: serde_json::Value = serde_json::from_str(r#"{"auditLogLimit":1000}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_audit_log_limit_zero() {
    let v: serde_json::Value = serde_json::from_str(r#"{"auditLogLimit":0}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_pass_audit_log_limit_absent() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

// ---------------------------------------------------------------------------
// auditLogLimit — error cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_fail_audit_log_limit_negative() {
    let v: serde_json::Value = serde_json::from_str(r#"{"auditLogLimit":-1}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("auditLogLimit must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_session_fail_audit_log_limit_not_number() {
    let v: serde_json::Value = serde_json::from_str(r#"{"auditLogLimit":"abc"}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("auditLogLimit must be a number"),
        "error: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// Combined fields
// ---------------------------------------------------------------------------

#[test]
fn test_validate_session_pass_both_plan_archive_days_and_audit_log_limit() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"planArchiveDays":30,"auditLogLimit":500}"#).unwrap();
    assert!(validate_session(&v).is_ok());
}

#[test]
fn test_validate_session_fail_plan_archive_days_invalid_with_valid_audit_log_limit() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"planArchiveDays":-1,"auditLogLimit":500}"#).unwrap();
    let err = validate_session(&v).unwrap_err();
    assert!(
        err.contains("planArchiveDays must be non-negative"),
        "error: {}",
        err
    );
}
