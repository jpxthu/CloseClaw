//! Tests for system.cron.schedule validation.

use crate::validators::validate_system;

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

#[test]
fn test_validate_system_cron_schedule_invalid() {
    let cases = [
        r#"{"cron":{"schedule":"abc xyz"}}"#,
        r#"{"cron":{"schedule":"* * *"}}"#,
        r#"{"cron":{"schedule":"0 */6 * * *"}}"#,
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
