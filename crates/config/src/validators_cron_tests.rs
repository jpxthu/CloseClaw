//! Tests for system.cron.schedule validation.

use crate::validators::validate_system;

// ---------------------------------------------------------------------------
// Normal path: valid 6-field cron expressions (seconds format)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_system_cron_schedule_valid() {
    let cases = [
        r#"{"cron":{"enabled":true,"schedule":"0 */6 * * * *"}}"#,
        r#"{"cron":{"enabled":true,"schedule":"0 30 3 * * *"}}"#,
        r#"{"cron":{"enabled":true,"schedule":"0 0 * * * *"}}"#,
    ];
    for json in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_system(&v).is_ok(), "json={}", json);
    }
}

// ---------------------------------------------------------------------------
// Normal path: schedule is None, absent, empty, or null → backward compatible
// ---------------------------------------------------------------------------

#[test]
fn test_validate_system_cron_schedule_none_or_absent_passes() {
    let cases = [
        r#"{"cron":{"enabled":true}}"#,
        r#"{"cron":{"enabled":true,"schedule":null}}"#,
        r#"{"cron":{"enabled":true,"schedule":""}}"#,
        r#"{}"#,
    ];
    for json in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_system(&v).is_ok(), "json={}", json);
    }
}

// ---------------------------------------------------------------------------
// Error path: invalid cron expressions
// ---------------------------------------------------------------------------

#[test]
fn test_validate_system_cron_schedule_invalid() {
    let cases = [
        r#"{"cron":{"schedule":"abc xyz"}}"#,
        r#"{"cron":{"schedule":"* * *"}}"#,
    ];
    for json in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_system(&v).unwrap_err();
        assert!(
            err.contains("system.cron.schedule must be a valid cron expression"),
            "json={}: error: {}",
            json,
            err
        );
    }
}

#[test]
fn test_validate_system_cron_schedule_not_string() {
    let cases = [
        r#"{"cron":{"schedule":123}}"#,
        r#"{"cron":{"schedule":true}}"#,
    ];
    for json in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_system(&v).unwrap_err();
        assert!(
            err.contains("system.cron.schedule must be a string"),
            "json={}: error: {}",
            json,
            err
        );
    }
}

// ---------------------------------------------------------------------------
// Boundary: special character combinations (comma, range, step)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_system_cron_schedule_special_chars_valid() {
    let cases = [
        // All wildcards (every second)
        r#"{"cron":{"schedule":"* * * * * *"}}"#,
        // Comma: multiple values
        r#"{"cron":{"schedule":"0,30 * * * * *"}}"#,
        // Range: 0-5
        r#"{"cron":{"schedule":"0 0-5 * * * *"}}"#,
        // Step: every 15 minutes
        r#"{"cron":{"schedule":"0 */15 * * * *"}}"#,
        // Combined: comma + step
        r#"{"cron":{"schedule":"0 0,15,30,45 * * * *"}}"#,
        // Range + step
        r#"{"cron":{"schedule":"0 0-23/2 * * * *"}}"#,
    ];
    for json in cases {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_system(&v).is_ok(), "json={}", json);
    }
}

#[test]
fn test_validate_system_cron_schedule_too_few_fields() {
    // 4 fields → invalid
    let v: serde_json::Value = serde_json::from_str(r#"{"cron":{"schedule":"* * * *"}}"#).unwrap();
    let err = validate_system(&v).unwrap_err();
    assert!(
        err.contains("system.cron.schedule must be a valid cron expression"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_system_cron_schedule_non_numeric() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"cron":{"schedule":"abc * * * * *"}}"#).unwrap();
    let err = validate_system(&v).unwrap_err();
    assert!(
        err.contains("system.cron.schedule must be a valid cron expression"),
        "error: {}",
        err
    );
}
