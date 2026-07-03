//! Unit tests for NormalizedMessage and related IM plugin types.

use crate::im_plugin::{MediaRef, MessageType, NormalizedMessage};

fn make_normalized(account_id: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_111".into(),
        peer_id: "oc_chat".into(),
        content: "hello".into(),
        timestamp: 1700000000000,
        message_type: MessageType::Text,
        media_refs: vec![],
        quoted_message: None,
        thread_id: None,
        account_id: account_id.into(),
    }
}

#[test]
fn test_normalized_account_id_is_string_not_option() {
    let msg = make_normalized("acct_1");
    assert_eq!(msg.account_id, "acct_1");
}

#[test]
fn test_normalized_account_id_empty_string_allowed() {
    let msg = make_normalized("");
    assert!(msg.account_id.is_empty());
}

#[test]
fn test_normalized_no_card_action_field() {
    let msg = make_normalized("a");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("card_action"));
}

#[test]
fn test_normalized_message_type_defaults_to_text() {
    let json = r#"{
        "platform": "p",
        "sender_id": "s",
        "peer_id": "r",
        "content": "x",
        "timestamp": 0
    }"#;
    let msg: NormalizedMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.message_type, MessageType::Text);
}

#[test]
fn test_normalized_roundtrip() {
    let mut msg = make_normalized("tenant_42");
    msg.message_type = MessageType::Image;
    msg.media_refs = vec![MediaRef {
        key: "file_abc".into(),
        url: "https://example.com/file_abc".into(),
    }];
    msg.thread_id = Some("t_99".into());

    let json = serde_json::to_string(&msg).unwrap();
    let de: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(de.account_id, "tenant_42");
    assert_eq!(de.message_type, MessageType::Image);
    assert_eq!(de.media_refs.len(), 1);
    assert_eq!(de.media_refs[0].key, "file_abc");
    assert_eq!(de.thread_id.as_deref(), Some("t_99"));
}

#[test]
fn test_normalized_quoted_message_roundtrip() {
    let mut msg = make_normalized("a");
    msg.quoted_message = Some("quoted text".into());

    let json = serde_json::to_string(&msg).unwrap();
    let de: NormalizedMessage = serde_json::from_str(&json).unwrap();
    let q = de.quoted_message.unwrap();
    assert_eq!(q, "quoted text");
}

#[test]
fn test_normalized_optional_fields_absent() {
    let json = r#"{
        "platform": "d",
        "sender_id": "1",
        "peer_id": "2",
        "content": "c",
        "timestamp": 0,
        "account_id": "x"
    }"#;
    let msg: NormalizedMessage = serde_json::from_str(json).unwrap();
    assert!(msg.media_refs.is_empty());
    assert!(msg.quoted_message.is_none());
    assert!(msg.thread_id.is_none());
}
