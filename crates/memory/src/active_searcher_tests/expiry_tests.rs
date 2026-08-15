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
    let ev = insert_event_with_expiry(&conn, "test event", 1000, "sess-1", 1000);

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev]).unwrap();
    assert_eq!(get_expires_at(&conn, ev), 1000 + 7 * 86400);
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
    assert_eq!(get_expires_at(&conn, ev), 5000, "extension_days=0 should not change");
}

#[test]
fn test_extend_event_expiry_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let ev1 = insert_event_with_expiry(&conn, "event 1", 1000, "sess-1", 1000);
    let ev2 = insert_event_with_expiry(&conn, "event 2", 2000, "sess-1", 2000);
    let ev3 = insert_event_with_expiry(&conn, "event 3", 3000, "sess-1", 3000);

    let config = ActiveSearcherConfig {
        injection_extension_days: 14,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev1, ev2, ev3]).unwrap();
    assert_eq!(get_expires_at(&conn, ev1), 1000 + 14 * 86400);
    assert_eq!(get_expires_at(&conn, ev2), 2000 + 14 * 86400);
    assert_eq!(get_expires_at(&conn, ev3), 3000 + 14 * 86400);
}

#[test]
fn test_extend_event_expiry_mixed_zero_and_positive() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = create_test_db(tmp.path());
    let ev1 = insert_event_with_expiry(&conn, "with expiry", 1000, "sess-1", 1000);
    let ev2 = insert_event_with_expiry(&conn, "no expiry", 2000, "sess-1", 0);

    let config = ActiveSearcherConfig {
        injection_extension_days: 7,
        ..Default::default()
    };
    let searcher = ActiveSearcher::new(tmp.path().join("test.db"), config);

    searcher.extend_event_expiry(&[ev1, ev2]).unwrap();
    assert_eq!(get_expires_at(&conn, ev1), 1000 + 7 * 86400);
    assert_eq!(get_expires_at(&conn, ev2), 0, "expires_at=0 should stay 0");
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
