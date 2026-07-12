//! Unit tests verifying the standalone `compute_session_key` function.

use super::*;

/// Same routing fields + same timestamp → identical key (deterministic).
#[test]
fn test_same_timestamp_same_fields_same_key() {
    let key1 = compute_session_key("feishu", "alice", "bob", Some("acc1"), 1_700_000_000_000);
    let key2 = compute_session_key("feishu", "alice", "bob", Some("acc1"), 1_700_000_000_000);
    assert_eq!(key1, key2, "same inputs must produce identical key");
}

/// Same routing fields + different timestamp → different key (timestamp is in hash input).
#[test]
fn test_different_timestamp_different_key() {
    let key1 = compute_session_key("feishu", "alice", "bob", Some("acc1"), 1_700_000_000_000);
    let key2 = compute_session_key("feishu", "alice", "bob", Some("acc1"), 1_700_000_000_001);
    assert_ne!(
        key1, key2,
        "different timestamps must produce different keys"
    );
}

/// Keys have the format `{timestamp_ms}-{64 hex chars}`.
#[test]
fn test_key_format() {
    let key = compute_session_key("feishu", "alice", "bob", Some("acc1"), 1_700_000_000_000);
    let parts: Vec<&str> = key.splitn(2, '-').collect();
    assert_eq!(parts.len(), 2, "key must have exactly one '-' separator");
    assert_eq!(parts[0], "1700000000000", "timestamp prefix mismatch");
    assert_eq!(parts[1].len(), 64, "hash must be 64 hex chars");
    assert!(
        parts[1].chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be valid hex"
    );
}

/// Timestamp sensitivity: different timestamps produce different hash parts.
#[test]
fn test_timestamp_affects_hash() {
    let key_early = compute_session_key("ch_x", "a", "b", None, 1000);
    let key_late = compute_session_key("ch_x", "a", "b", None, 999999);
    let hash_early = &key_early[key_early.find('-').unwrap() + 1..];
    let hash_late = &key_late[key_late.find('-').unwrap() + 1..];
    assert_ne!(
        hash_early, hash_late,
        "routing hash must differ when timestamp_ms changes"
    );
    assert!(key_early.starts_with("1000-"));
    assert!(key_late.starts_with("999999-"));
}

/// account_id=None uses "default" in the hash.
#[test]
fn test_account_id_none_uses_default() {
    let key1 = compute_session_key("ch_x", "a", "b", None, 0);
    let key2 = compute_session_key("ch_x", "a", "b", Some("default"), 0);
    assert_eq!(
        key1, key2,
        "None and Some('default') should produce the same key"
    );
}

/// Different account_ids produce different keys.
#[test]
fn test_different_account_ids_different_keys() {
    let key1 = compute_session_key("ch_x", "a", "b", Some("acc1"), 0);
    let key2 = compute_session_key("ch_x", "a", "b", Some("acc2"), 0);
    assert_ne!(
        key1, key2,
        "different account_ids must produce different keys"
    );
}

/// Different senders produce different keys.
#[test]
fn test_different_senders_different_keys() {
    let key1 = compute_session_key("ch_x", "alice", "b", None, 0);
    let key2 = compute_session_key("ch_x", "bob", "b", None, 0);
    assert_ne!(key1, key2, "different senders must produce different keys");
}

/// Different channels produce different keys.
#[test]
fn test_different_channels_different_keys() {
    let key1 = compute_session_key("feishu", "a", "b", None, 0);
    let key2 = compute_session_key("discord", "a", "b", None, 0);
    assert_ne!(key1, key2, "different channels must produce different keys");
}

/// Timestamp sensitivity across multiple calls.
#[test]
fn test_includes_timestamp_sensitivity() {
    let k1 = compute_session_key("feishu", "a", "b", None, 100);
    let k2 = compute_session_key("feishu", "a", "b", None, 200);
    assert_ne!(k1, k2, "different timestamps must produce different keys");
}
