//! Tests for the standalone `compute_session_key` function.

use crate::compute_session_key;

// ── compute_session_key unit tests ─────────────────────────────────────────

#[test]
fn test_compute_session_key_format() {
    let key = compute_session_key("feishu", "ou_u1", "ag", Some("acc1"), 1_700_000_000_000);
    let parts: Vec<&str> = key.splitn(2, '-').collect();
    assert_eq!(parts.len(), 2, "key must have exactly one '-' separator");
    assert_eq!(parts[0], "1700000000000", "timestamp prefix mismatch");
    assert_eq!(parts[1].len(), 64, "hash must be 64 hex chars");
    assert!(
        parts[1].chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be valid hex"
    );
}

#[test]
fn test_compute_session_key_deterministic() {
    let key1 = compute_session_key("feishu", "ou_u1", "ag", Some("acc1"), 100);
    let key2 = compute_session_key("feishu", "ou_u1", "ag", Some("acc1"), 100);
    assert_eq!(key1, key2, "same inputs must produce identical key");
}

#[test]
fn test_compute_session_key_different_senders() {
    let key1 = compute_session_key("feishu", "ou_u1", "ag", None, 100);
    let key2 = compute_session_key("feishu", "ou_u2", "ag", None, 100);
    assert_ne!(key1, key2, "different senders must produce different keys");
}

#[test]
fn test_compute_session_key_different_accounts() {
    let key1 = compute_session_key("feishu", "ou_u1", "ag", Some("ta"), 100);
    let key2 = compute_session_key("feishu", "ou_u1", "ag", Some("tb"), 100);
    assert_ne!(
        key1, key2,
        "different account_ids must produce different keys"
    );
}

#[test]
fn test_compute_session_key_none_vs_default_account() {
    let key_none = compute_session_key("feishu", "ou_u1", "ag", None, 100);
    let key_default = compute_session_key("feishu", "ou_u1", "ag", Some("default"), 100);
    assert_eq!(
        key_none, key_default,
        "None and Some('default') should produce the same key"
    );
}

#[test]
fn test_compute_session_key_account_none_uses_default_string() {
    // Verify that account_id=None uses literal "default" in hash input
    let key_with_none = compute_session_key("ch", "a", "b", None, 0);
    let key_with_explicit = compute_session_key("ch", "a", "b", Some("default"), 0);
    assert_eq!(key_with_none, key_with_explicit);
}

#[test]
fn test_compute_session_key_different_channels() {
    let key1 = compute_session_key("feishu", "a", "b", None, 100);
    let key2 = compute_session_key("discord", "a", "b", None, 100);
    assert_ne!(key1, key2, "different channels must produce different keys");
}

#[test]
fn test_compute_session_key_timestamp_sensitivity() {
    let k1 = compute_session_key("feishu", "a", "b", None, 100);
    let k2 = compute_session_key("feishu", "a", "b", None, 200);
    assert_ne!(k1, k2, "different timestamps must produce different keys");
}
