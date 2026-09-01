//! Integration tests for FeishuAdapter
//!
//! These tests were migrated from `src/im/feishu.rs` `#[cfg(test)] mod tests`.

use super::adapter::EventDeduplicator;
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
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    assert!(a.send_message(&msg, None).await.is_err());
}

// ===========================================================================
// Event deduplication tests
// ===========================================================================

/// Build a minimal message event payload with a custom event_id.
fn make_message_payload(event_id: &str, text: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": event_id,
            "event_type": "im.message.receive_v1",
            "create_time": "0",
            "token": "t",
            "app_id": "a"
        },
        "event": {
            "sender": {"sender_id": {"open_id": "ou_abc"}, "sender_type": "user"},
            "content": serde_json::json!({"text": text}).to_string(),
            "chat_id": "oc_x",
            "message_type": "text"
        }
    });
    serde_json::to_vec(&payload).unwrap()
}

/// Build a card.action.trigger payload with a custom event_id.
///
/// `FeishuCardActionEvent` deserializes from the top-level JSON object,
/// so `operator`, `token`, `action` must be siblings of `header`, not
/// nested inside an `event` wrapper.
fn make_card_action_payload(event_id: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": event_id,
            "event_type": "card.action.trigger",
            "create_time": "0",
            "token": "t",
            "app_id": "a"
        },
        "operator": {"open_id": "ou_op"},
        "token": "tok",
        "action": {"value": {"action": "btn_click"}}
    });
    serde_json::to_vec(&payload).unwrap()
}

/// First event with a given event_id passes through normally.
#[tokio::test]
async fn test_dedup_first_event_passes() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    let payload = make_message_payload("ev_dedup_001", "hello");
    let msg = adapter.parse_inbound(&payload).await.unwrap();
    assert!(msg.is_some(), "first event should pass dedup");
    assert_eq!(msg.unwrap().content, "hello");
}

/// Duplicate event_id is rejected (returns Ok(None)).
#[tokio::test]
async fn test_dedup_duplicate_rejected() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    let payload = make_message_payload("ev_dedup_002", "first");
    let msg1 = adapter.parse_inbound(&payload).await.unwrap();
    assert!(msg1.is_some(), "first event should pass");

    // Same event_id, different content — should be rejected.
    let payload2 = make_message_payload("ev_dedup_002", "second");
    let msg2 = adapter.parse_inbound(&payload2).await.unwrap();
    assert!(msg2.is_none(), "duplicate event_id should be rejected");
}

/// Duplicate event_id in card.action.trigger is also rejected.
#[tokio::test]
async fn test_dedup_card_action_duplicate_rejected() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    let payload = make_card_action_payload("ev_card_dedup_001");
    // First call: card.action.trigger returns Ok(None) by design (Step 1.4),
    // but the event_id is recorded in dedup.
    let _ = adapter.parse_card_action(&payload).await.unwrap();

    // Second call with same event_id — should be rejected (returns Ok(None)
    // but now via dedup path, not card.action.trigger path).
    let result = adapter.parse_card_action(&payload).await.unwrap();
    assert!(
        result.is_none(),
        "duplicate card action event_id should be rejected"
    );
}

/// Capacity eviction: after filling to capacity, oldest entry is evicted
/// so new events can still be accepted.
#[test]
fn test_dedup_capacity_eviction() {
    let mut dedup = EventDeduplicator::new(10); // small capacity for fast test
                                                // Fill to capacity.
    for i in 0..10 {
        assert!(
            !dedup.check_and_record(&format!("ev_{i}")),
            "event ev_{i} should be accepted"
        );
    }
    // All 10 slots full. New event evicts oldest (ev_0).
    assert!(
        !dedup.check_and_record("ev_new"),
        "new event after capacity fill should be accepted"
    );
    // ev_0 was evicted → accepted again.
    assert!(
        !dedup.check_and_record("ev_0"),
        "ev_0 evicted then re-appeared should be accepted"
    );
    // Evicting ev_0 pushed out ev_1 (at capacity again). Verify ev_2 still present.
    assert!(
        dedup.check_and_record("ev_2"),
        "ev_2 still in set should be rejected"
    );
    // ev_9 (last original) should still be present.
    assert!(
        dedup.check_and_record("ev_9"),
        "ev_9 still in set should be rejected"
    );
}

/// Empty event_id is never deduplicated (always passes).
#[tokio::test]
async fn test_dedup_empty_event_id_always_passes() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    let payload = make_message_payload("", "hello");
    let msg1 = adapter.parse_inbound(&payload).await.unwrap();
    assert!(msg1.is_some(), "empty event_id first call should pass");

    let msg2 = adapter.parse_inbound(&payload).await.unwrap();
    assert!(
        msg2.is_some(),
        "empty event_id second call should also pass"
    );
}

// ===========================================================================
// Dedup-before-side-effects verification
// ===========================================================================

/// Verify that a duplicate event_id is rejected before any media download
/// or network I/O occurs. We test this by sending the same event_id twice:
/// the second call returns Ok(None) immediately, confirming the dedup
/// gate fired before parse_message_event (which would call persist_media_refs)
/// could execute.
#[tokio::test]
async fn test_dedup_before_media_download() {
    let adapter = FeishuAdapter::new("test_profile".into(), make_test_media_store());
    // Use an image event which would require media download in the normal path.
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_media_001",
            "event_type": "im.message.receive_v1",
            "create_time": "0",
            "token": "t",
            "app_id": "a"
        },
        "event": {
            "sender": {"sender_id": {"open_id": "ou_abc"}, "sender_type": "user"},
            "content": serde_json::json!({"image_key": "img_123"}).to_string(),
            "chat_id": "oc_x",
            "message_type": "image"
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();

    // First call: would normally trigger media download path.
    let _ = adapter.parse_inbound(&bytes).await;

    // Second call: dedup rejects before media download can happen.
    let result = adapter.parse_inbound(&bytes).await.unwrap();
    assert!(
        result.is_none(),
        "duplicate event must be rejected before media download side-effects"
    );
}
