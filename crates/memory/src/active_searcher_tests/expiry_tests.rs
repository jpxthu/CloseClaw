use closeclaw_config::agents::{
    default_forgetting_injection_extension_days, ForgettingConfig, MemoryConfig,
};

use crate::active_searcher::{ActiveSearcher, ActiveSearcherConfig};

use super::{create_test_db, get_expires_at, insert_event_with_expiry};

// ── extend_event_expiry tests ────────────────────────────────────────────

#[test]
fn test_extend_event_expiry_basic() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    // Use a future expires_at so MAX(expires_at, now) == expires_at
    let future_expires = 2_000_000_000;
    let ev = insert_event_with_expiry(&conn, "test event", 1000, "sess-1", future_expires);

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev]).unwrap();
    assert_eq!(get_expires_at(&conn, ev), future_expires + 7 * 86400);
}

#[test]
fn test_extend_event_expiry_does_not_touch_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    // expires_at = 0 means forgetting disabled
    let ev = insert_event_with_expiry(&conn, "test event", 1000, "sess-1", 0);

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev]).unwrap();
    assert_eq!(get_expires_at(&conn, ev), 0, "expires_at=0 should stay 0");
}

#[test]
fn test_extend_event_expiry_nonexistent_id() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    // Should not error even if IDs don't exist
    searcher.extend_event_expiry(&[999, 1000]).unwrap();
}

#[test]
fn test_extend_event_expiry_empty_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let _conn = create_test_db(tmp.path());

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[]).unwrap();
}

#[test]
fn test_extend_event_expiry_zero_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let ev = insert_event_with_expiry(&conn, "test event", 1000, "sess-1", 5000);

    let config = ActiveSearcherConfig {
        injection_extension_days: 0,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev]).unwrap();
    assert_eq!(
        get_expires_at(&conn, ev),
        5000,
        "extension_days=0 should not change"
    );
}

#[test]
fn test_extend_event_expiry_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    // Use future expires_at values so MAX(expires_at, now) == expires_at
    let future_expires1 = 2_000_000_000;
    let future_expires2 = 2_000_001_000;
    let future_expires3 = 2_000_002_000;
    let ev1 = insert_event_with_expiry(&conn, "event 1", 1000, "sess-1", future_expires1);
    let ev2 = insert_event_with_expiry(&conn, "event 2", 2000, "sess-1", future_expires2);
    let ev3 = insert_event_with_expiry(&conn, "event 3", 3000, "sess-1", future_expires3);

    let config = ActiveSearcherConfig {
        injection_extension_days: 14,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev1, ev2, ev3]).unwrap();
    assert_eq!(get_expires_at(&conn, ev1), future_expires1 + 14 * 86400);
    assert_eq!(get_expires_at(&conn, ev2), future_expires2 + 14 * 86400);
    assert_eq!(get_expires_at(&conn, ev3), future_expires3 + 14 * 86400);
}

#[test]
fn test_extend_event_expiry_mixed_zero_and_positive() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    // Use a future expires_at so MAX(expires_at, now) == expires_at
    let future_expires = 2_000_000_000;
    let ev1 = insert_event_with_expiry(&conn, "with expiry", 1000, "sess-1", future_expires);
    let ev2 = insert_event_with_expiry(&conn, "no expiry", 2000, "sess-1", 0);

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev1, ev2]).unwrap();
    assert_eq!(get_expires_at(&conn, ev1), future_expires + 7 * 86400);
    assert_eq!(get_expires_at(&conn, ev2), 0, "expires_at=0 should stay 0");
}

#[test]
fn test_extend_event_expiry_expired_event_not_resurrected() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    // Set expires_at to a past time (event is expired)
    let past_expires = 100;
    let ev = insert_event_with_expiry(&conn, "expired event", 1000, "sess-1", past_expires);

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev]).unwrap();

    let new_expires = get_expires_at(&conn, ev);
    // With MAX保护: MAX(past_expires, now) + 7d = now + 7d
    // Without保护 (old bug): past_expires + 7d = 100 + 604800 = 604900
    // The new value should be much larger than old_value (now > past_expires)
    let old_value_without_fix = past_expires + 7 * 86400;
    assert!(
        new_expires > old_value_without_fix,
        "expired event should be extended from now, not from past expires_at; \
         got {new_expires}, old (buggy) value would be {old_value_without_fix}"
    );
}

// ── Config parsing tests ─────────────────────────────────────────────────

#[test]
fn test_config_injection_extension_days_default() {
    let config = ActiveSearcherConfig::default();
    assert_eq!(
        config.injection_extension_days,
        default_forgetting_injection_extension_days()
    );
}

#[test]
fn test_from_agent_config_with_forgetting() {
    let mut memory = MemoryConfig::default();
    memory.search.enabled = Some(true);
    let forgetting = ForgettingConfig {
        initial_ttl_days: Some(30),
        reidentify_extension_days: None,
        injection_extension_days: Some(30),
    };

    let config =
        ActiveSearcherConfig::from_agent_config(Some("model"), Some(&memory), Some(&forgetting));
    assert!(config.is_some());
    assert_eq!(config.unwrap().injection_extension_days, 30);
}

#[test]
fn test_from_agent_config_without_forgetting() {
    let mut memory = MemoryConfig::default();
    memory.search.enabled = Some(true);

    let config = ActiveSearcherConfig::from_agent_config(Some("model"), Some(&memory), None);
    assert!(config.is_some());
    assert_eq!(
        config.unwrap().injection_extension_days,
        default_forgetting_injection_extension_days()
    );
}

#[test]
fn test_from_agent_config_search_disabled() {
    let memory = MemoryConfig::default(); // search.enabled = None → default false

    let config = ActiveSearcherConfig::from_agent_config(Some("model"), Some(&memory), None);
    assert!(config.is_none(), "search disabled should return None");
}

#[test]
fn test_config_from_json_with_forgetting() {
    let json = r#"{
        "forgetting": {
            "injectionExtensionDays": 30
        }
    }"#;
    let memory: MemoryConfig = serde_json::from_str(json).unwrap();
    assert_eq!(memory.forgetting.injection_extension_days, Some(30));
}

#[test]
fn test_config_from_json_defaults() {
    let json = r#"{}"#;
    let memory: MemoryConfig = serde_json::from_str(json).unwrap();
    assert_eq!(memory.forgetting.injection_extension_days, None);
}
