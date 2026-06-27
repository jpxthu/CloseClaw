//! Unit tests for NormalizedMessage new fields (message_type, media_refs,
//! quoted_message) and their sub-structs (MediaRef, QuotedMessage).
//!
//! Covers:
//! - serde defaults for missing fields in JSON
//! - roundtrip serialization for NormalizedMessage with new fields
//! - MediaRef and QuotedMessage independent roundtrip
//! - edge cases: empty media_refs, quoted_message None vs present

use crate::normalized::{MediaRef, NormalizedMessage, QuotedMessage};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_minimal_json() -> serde_json::Value {
    serde_json::json!({
        "platform": "feishu",
        "sender_id": "ou_abc",
        "peer_id": "oc_xyz",
        "content": "hi",
        "timestamp": 1000
    })
}

// ---------------------------------------------------------------------------
// Serde defaults
// ---------------------------------------------------------------------------

#[test]
fn test_deserialize_defaults_message_type() {
    let json = make_minimal_json();
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert_eq!(msg.message_type, "text");
}

#[test]
fn test_deserialize_defaults_media_refs_empty() {
    let json = make_minimal_json();
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert!(msg.media_refs.is_empty());
}

#[test]
fn test_deserialize_defaults_quoted_message_none() {
    let json = make_minimal_json();
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert!(msg.quoted_message.is_none());
}

#[test]
fn test_deserialize_all_defaults_present() {
    let json = make_minimal_json();
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert_eq!(msg.platform, "feishu");
    assert_eq!(msg.sender_id, "ou_abc");
    assert_eq!(msg.peer_id, "oc_xyz");
    assert_eq!(msg.content, "hi");
    assert_eq!(msg.timestamp, 1000);
    assert_eq!(msg.message_type, "text");
    assert!(msg.media_refs.is_empty());
    assert!(msg.quoted_message.is_none());
    assert!(msg.thread_id.is_none());
    assert!(msg.account_id.is_none());
    assert_eq!(msg.card_action, None);
}

// ---------------------------------------------------------------------------
// message_type override
// ---------------------------------------------------------------------------

#[test]
fn test_deserialize_explicit_message_type() {
    let mut json = make_minimal_json();
    json["message_type"] = serde_json::json!("image");
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert_eq!(msg.message_type, "image");
}

#[test]
fn test_roundtrip_message_type_non_default() {
    let msg = NormalizedMessage {
        platform: "test".into(),
        sender_id: "s".into(),
        peer_id: "p".into(),
        content: String::new(),
        timestamp: 0,
        message_type: "file".into(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: None,
        card_action: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.message_type, "file");
}

// ---------------------------------------------------------------------------
// MediaRef
// ---------------------------------------------------------------------------

#[test]
fn test_media_ref_roundtrip() {
    let r = MediaRef {
        key: "img_v2_abc".into(),
        url: "https://example.com/img.png".into(),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: MediaRef = serde_json::from_str(&json).unwrap();
    assert_eq!(back.key, r.key);
    assert_eq!(back.url, r.url);
}

#[test]
fn test_deserialize_media_refs_present() {
    let mut json = make_minimal_json();
    json["media_refs"] = serde_json::json!([
        {"key": "k1", "url": "http://a.com/1"},
        {"key": "k2", "url": "http://a.com/2"}
    ]);
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert_eq!(msg.media_refs.len(), 2);
    assert_eq!(msg.media_refs[0].key, "k1");
    assert_eq!(msg.media_refs[1].url, "http://a.com/2");
}

#[test]
fn test_roundtrip_message_with_media_refs() {
    let msg = NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "s".into(),
        peer_id: "p".into(),
        content: String::new(),
        timestamp: 0,
        message_type: "image".into(),
        media_refs: vec![
            MediaRef {
                key: "k1".into(),
                url: "http://a.com/1".into(),
            },
            MediaRef {
                key: "k2".into(),
                url: "http://a.com/2".into(),
            },
        ],
        quoted_message: None,
        thread_id: None,
        account_id: None,
        card_action: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.media_refs.len(), 2);
    assert_eq!(back.media_refs[0].key, "k1");
    assert_eq!(back.media_refs[1].url, "http://a.com/2");
}

// ---------------------------------------------------------------------------
// QuotedMessage
// ---------------------------------------------------------------------------

#[test]
fn test_quoted_message_roundtrip() {
    let q = QuotedMessage {
        content: "original text".into(),
        sender_id: Some("ou_sender".into()),
    };
    let json = serde_json::to_string(&q).unwrap();
    let back: QuotedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.content, "original text");
    assert_eq!(back.sender_id.as_deref(), Some("ou_sender"));
}

#[test]
fn test_quoted_message_sender_id_none() {
    let q = QuotedMessage {
        content: "reply".into(),
        sender_id: None,
    };
    let json = serde_json::to_string(&q).unwrap();
    let back: QuotedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.content, "reply");
    assert!(back.sender_id.is_none());
}

#[test]
fn test_deserialize_quoted_message_present() {
    let mut json = make_minimal_json();
    json["quoted_message"] = serde_json::json!({
        "content": "quoted text",
        "sender_id": "ou_orig"
    });
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    let q = msg.quoted_message.expect("expected quoted_message");
    assert_eq!(q.content, "quoted text");
    assert_eq!(q.sender_id.as_deref(), Some("ou_orig"));
}

#[test]
fn test_deserialize_quoted_message_without_sender_id() {
    let mut json = make_minimal_json();
    json["quoted_message"] = serde_json::json!({
        "content": "orphan quote"
    });
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    let q = msg.quoted_message.expect("expected quoted_message");
    assert_eq!(q.content, "orphan quote");
    assert!(q.sender_id.is_none());
}

#[test]
fn test_roundtrip_message_with_quoted() {
    let msg = NormalizedMessage {
        platform: "discord".into(),
        sender_id: "s".into(),
        peer_id: "p".into(),
        content: "my reply".into(),
        timestamp: 42,
        message_type: "text".into(),
        media_refs: vec![],
        quoted_message: Some(QuotedMessage {
            content: "original".into(),
            sender_id: Some("orig_sender".into()),
        }),
        thread_id: None,
        account_id: None,
        card_action: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: NormalizedMessage = serde_json::from_str(&json).unwrap();
    let q = back.quoted_message.unwrap();
    assert_eq!(q.content, "original");
    assert_eq!(q.sender_id.as_deref(), Some("orig_sender"));
}

// ---------------------------------------------------------------------------
// Combined: all new fields populated
// ---------------------------------------------------------------------------

#[test]
fn test_roundtrip_all_new_fields_populated() {
    let msg = NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_user".into(),
        peer_id: "oc_group".into(),
        content: "replied with image".into(),
        timestamp: 1_700_000_000_000,
        message_type: "image".into(),
        media_refs: vec![MediaRef {
            key: "img_key_001".into(),
            url: "https://cdn.feishu.cn/img.png".into(),
        }],
        quoted_message: Some(QuotedMessage {
            content: "check this out".into(),
            sender_id: Some("ou_other".into()),
        }),
        thread_id: Some("omt_123".into()),
        account_id: Some("tenant_999".into()),
        card_action: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: NormalizedMessage = serde_json::from_str(&json).unwrap();

    assert_eq!(back.message_type, "image");
    assert_eq!(back.media_refs.len(), 1);
    assert_eq!(back.media_refs[0].key, "img_key_001");
    assert_eq!(back.media_refs[0].url, "https://cdn.feishu.cn/img.png");
    let q = back.quoted_message.unwrap();
    assert_eq!(q.content, "check this out");
    assert_eq!(q.sender_id.as_deref(), Some("ou_other"));
    assert_eq!(back.thread_id.as_deref(), Some("omt_123"));
    assert_eq!(back.account_id.as_deref(), Some("tenant_999"));
}

// ---------------------------------------------------------------------------
// Backward compatibility: JSON with only old fields still deserializes
// ---------------------------------------------------------------------------

#[test]
fn test_backward_compat_old_json_without_new_fields() {
    let json = serde_json::json!({
        "platform": "telegram",
        "sender_id": "tg_user",
        "peer_id": "tg_chat",
        "content": "legacy message",
        "timestamp": 1234567890,
        "thread_id": null,
        "account_id": null
    });
    let msg: NormalizedMessage = serde_json::from_value(json).expect("backward compat failed");
    assert_eq!(msg.message_type, "text");
    assert!(msg.media_refs.is_empty());
    assert!(msg.quoted_message.is_none());
    assert!(msg.card_action.is_none());
}

// ---------------------------------------------------------------------------
// Debug trait
// ---------------------------------------------------------------------------

#[test]
fn test_normalized_message_debug_contains_key_fields() {
    let msg = NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "s".into(),
        peer_id: "p".into(),
        content: "c".into(),
        timestamp: 0,
        message_type: "text".into(),
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: None,
        card_action: None,
    };
    let debug = format!("{:?}", msg);
    assert!(debug.contains("NormalizedMessage"));
    assert!(debug.contains("feishu"));
}

// ---------------------------------------------------------------------------
// Clone trait
// ---------------------------------------------------------------------------

#[test]
fn test_normalized_message_clone() {
    let msg = NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "s".into(),
        peer_id: "p".into(),
        content: "c".into(),
        timestamp: 0,
        message_type: "image".into(),
        media_refs: vec![MediaRef {
            key: "k".into(),
            url: "u".into(),
        }],
        quoted_message: Some(QuotedMessage {
            content: "q".into(),
            sender_id: Some("qsid".into()),
        }),
        thread_id: Some("t".into()),
        account_id: Some("a".into()),
        card_action: None,
    };
    let cloned = msg.clone();
    assert_eq!(cloned.platform, msg.platform);
    assert_eq!(cloned.message_type, msg.message_type);
    assert_eq!(cloned.media_refs.len(), 1);
    assert_eq!(cloned.quoted_message.as_ref().unwrap().content, "q");
}
