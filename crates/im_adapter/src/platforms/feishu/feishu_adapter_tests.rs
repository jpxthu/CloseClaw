//! Integration tests for FeishuAdapter
//!
//! These tests were migrated from `src/im/feishu.rs` `#[cfg(test)] mod tests`.

use super::FeishuAdapter;
use crate::media_store::MediaStore;
use crate::IMAdapter;
use closeclaw_gateway::Message;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

#[test]
fn test_feishu_adapter_name() {
    let adapter = FeishuAdapter::new("test_profile".to_string(), make_test_media_store());
    assert_eq!(adapter.name(), "feishu");
}

#[tokio::test]
async fn test_validate_signature_correct() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    // lark-cli event consume handles signature verification;
    // validate_signature always returns true.
    assert!(adapter.validate_signature("any", b"test").await);
}

#[tokio::test]
async fn test_validate_signature_incorrect() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    // lark-cli event consume handles signature verification;
    // validate_signature always returns true.
    assert!(adapter.validate_signature("wrong", b"test").await);
}

#[tokio::test]
async fn test_parse_inbound_valid() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {"event_id":"evt_1","event_type":"im.message.receive_v1","create_time":"0","token":"t","app_id":"a"},
        "event": {"sender":{"sender_id":{"open_id":"ou_abc"},"sender_type":"user"},"content":"{\"text\":\"hello\"}","chat_id":"oc_x","message_type":"text"}
    });
    let msg = adapter
        .parse_inbound(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap()
        .expect("expected Some(msg)");
    assert_eq!(msg.sender_id, "ou_abc");
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.account_id.as_str(), "ou_abc");
    assert_eq!(msg.platform, "feishu");
}

#[tokio::test]
async fn test_parse_inbound_invalid_json() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    assert!(adapter.parse_inbound(b"not json").await.is_err());
}

#[tokio::test]
async fn test_parse_inbound_empty_text() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    let payload = serde_json::json!({
        "schema":"2.0","header":{"event_id":"e2","event_type":"x","create_time":"0","token":"t","app_id":"a"},
        "event":{"sender":{"sender_id":{"open_id":"ou_x"},"sender_type":"user"},"content":"{\"other\":\"data\"}","chat_id":"oc_y","message_type":"text"}
    });
    let result = adapter
        .parse_inbound(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_error_cases() {
    let a = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    let msg = Message {
        id: "1".into(),
        from: "a".into(),
        to: "b".into(),
        content: "hi".into(),
        channel: "feishu".into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    assert!(a.send_message(&msg, None).await.is_err());
}
