//! Unit tests verifying that `timestamp_ms` participates in the session_key hash.

use super::*;

fn make_msg(from: &str, to: &str, channel: &str) -> Message {
    use std::collections::HashMap;
    Message {
        id: "msg_1".into(),
        from: from.into(),
        to: to.into(),
        content: "hi".into(),
        channel: channel.into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
    }
}

/// Same routing fields + same timestamp → identical key (deterministic).
#[test]
fn test_same_timestamp_same_fields_same_key() {
    let msg = make_msg("alice", "bob", "feishu");
    let key1 = DmScope::PerAccountChannelPeer.compute_session_key(
        "feishu",
        &msg,
        Some("acc1"),
        1_700_000_000_000,
    );
    let key2 = DmScope::PerAccountChannelPeer.compute_session_key(
        "feishu",
        &msg,
        Some("acc1"),
        1_700_000_000_000,
    );
    assert_eq!(key1, key2, "same inputs must produce identical key");
}

/// Same routing fields + different timestamp → different key (timestamp is in hash input).
#[test]
fn test_different_timestamp_different_key() {
    let msg = make_msg("alice", "bob", "feishu");
    let key1 = DmScope::PerAccountChannelPeer.compute_session_key(
        "feishu",
        &msg,
        Some("acc1"),
        1_700_000_000_000,
    );
    let key2 = DmScope::PerAccountChannelPeer.compute_session_key(
        "feishu",
        &msg,
        Some("acc1"),
        1_700_000_000_001,
    );
    assert_ne!(
        key1, key2,
        "different timestamps must produce different keys"
    );
}

/// Keys have the format `{timestamp_ms}-{64 hex chars}`.
#[test]
fn test_key_format() {
    let msg = make_msg("alice", "bob", "feishu");
    let key = DmScope::PerAccountChannelPeer.compute_session_key(
        "feishu",
        &msg,
        Some("acc1"),
        1_700_000_000_000,
    );
    let parts: Vec<&str> = key.splitn(2, '-').collect();
    assert_eq!(parts.len(), 2, "key must have exactly one '-' separator");
    assert_eq!(parts[0], "1700000000000", "timestamp prefix mismatch");
    assert_eq!(parts[1].len(), 64, "hash must be 64 hex chars");
    assert!(
        parts[1].chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be valid hex"
    );
}

// ── Per-scope variant timestamp sensitivity ──────────────────────────────────

#[test]
fn test_main_includes_timestamp() {
    let msg = make_msg("alice", "bob", "feishu");
    let k1 = DmScope::Main.compute_session_key("feishu", &msg, None, 100);
    let k2 = DmScope::Main.compute_session_key("feishu", &msg, None, 200);
    assert_ne!(k1, k2, "Main must be sensitive to timestamp_ms");
}

#[test]
fn test_per_peer_includes_timestamp() {
    let msg = make_msg("alice", "bob", "feishu");
    let k1 = DmScope::PerPeer.compute_session_key("feishu", &msg, None, 100);
    let k2 = DmScope::PerPeer.compute_session_key("feishu", &msg, None, 200);
    assert_ne!(k1, k2, "PerPeer must be sensitive to timestamp_ms");
}

#[test]
fn test_per_channel_peer_includes_timestamp() {
    let msg = make_msg("alice", "bob", "feishu");
    let k1 = DmScope::PerChannelPeer.compute_session_key("feishu", &msg, None, 100);
    let k2 = DmScope::PerChannelPeer.compute_session_key("feishu", &msg, None, 200);
    assert_ne!(k1, k2, "PerChannelPeer must be sensitive to timestamp_ms");
}

#[test]
fn test_per_account_channel_peer_includes_timestamp() {
    let msg = make_msg("alice", "bob", "feishu");
    let k1 = DmScope::PerAccountChannelPeer.compute_session_key("feishu", &msg, Some("acc1"), 100);
    let k2 = DmScope::PerAccountChannelPeer.compute_session_key("feishu", &msg, Some("acc1"), 200);
    assert_ne!(
        k1, k2,
        "PerAccountChannelPeer must be sensitive to timestamp_ms"
    );
}

#[test]
fn test_per_channel_sender_includes_timestamp() {
    let msg = make_msg("alice", "bob", "feishu");
    let k1 = DmScope::PerChannelSender.compute_session_key("feishu", &msg, None, 100);
    let k2 = DmScope::PerChannelSender.compute_session_key("feishu", &msg, None, 200);
    assert_ne!(k1, k2, "PerChannelSender must be sensitive to timestamp_ms");
}
