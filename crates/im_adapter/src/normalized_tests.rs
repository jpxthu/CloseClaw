//! Unit tests for NormalizedMessage new fields (message_type, media_refs)
//! and sub-structs (MediaRef).
//!
//! Covers:
//! - serde defaults for missing fields in JSON
//! - roundtrip serialization for NormalizedMessage with new fields
//! - MediaRef independent roundtrip
//! - edge cases: empty media_refs

use closeclaw_common::{MediaRef, MessageType, NormalizedMessage};

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
    assert_eq!(msg.message_type, MessageType::Text);
}

#[test]
fn test_deserialize_defaults_media_refs_empty() {
    let json = make_minimal_json();
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert!(msg.media_refs.is_empty());
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
    assert_eq!(msg.message_type, MessageType::Text);
    assert!(msg.media_refs.is_empty());
    assert!(msg.thread_id.is_none());
    assert!(msg.account_id.is_empty());
}

// ---------------------------------------------------------------------------
// message_type override
// ---------------------------------------------------------------------------

#[test]
fn test_deserialize_explicit_message_type() {
    let mut json = make_minimal_json();
    json["message_type"] = serde_json::json!("image");
    let msg: NormalizedMessage = serde_json::from_value(json).expect("deserialization failed");
    assert_eq!(msg.message_type, MessageType::Image);
}

#[test]
fn test_roundtrip_message_type_non_default() {
    let msg = NormalizedMessage {
        platform: "test".into(),
        sender_id: "s".into(),
        peer_id: "p".into(),
        content: String::new(),
        timestamp: 0,
        message_type: MessageType::File,
        media_refs: vec![],
        thread_id: None,
        account_id: String::new(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.message_type, MessageType::File);
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
        message_type: MessageType::Image,
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
        thread_id: None,
        account_id: String::new(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.media_refs.len(), 2);
    assert_eq!(back.media_refs[0].key, "k1");
    assert_eq!(back.media_refs[1].url, "http://a.com/2");
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
        message_type: MessageType::Image,
        media_refs: vec![MediaRef {
            key: "img_key_001".into(),
            url: "https://cdn.feishu.cn/img.png".into(),
        }],
        thread_id: Some("omt_123".into()),
        account_id: "tenant_999".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: NormalizedMessage = serde_json::from_str(&json).unwrap();

    assert_eq!(back.message_type, MessageType::Image);
    assert_eq!(back.media_refs.len(), 1);
    assert_eq!(back.media_refs[0].key, "img_key_001");
    assert_eq!(back.media_refs[0].url, "https://cdn.feishu.cn/img.png");
    assert_eq!(back.thread_id.as_deref(), Some("omt_123"));
    assert_eq!(back.account_id, "tenant_999");
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
        "thread_id": null
    });
    let msg: NormalizedMessage = serde_json::from_value(json).expect("backward compat failed");
    assert_eq!(msg.message_type, MessageType::Text);
    assert!(msg.media_refs.is_empty());
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
        message_type: MessageType::Text,
        media_refs: vec![],
        thread_id: None,
        account_id: String::new(),
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
        message_type: MessageType::Image,
        media_refs: vec![MediaRef {
            key: "k".into(),
            url: "u".into(),
        }],
        thread_id: Some("t".into()),
        account_id: "a".into(),
    };
    let cloned = msg.clone();
    assert_eq!(cloned.platform, msg.platform);
    assert_eq!(cloned.message_type, msg.message_type);
    assert_eq!(cloned.media_refs.len(), 1);
}
