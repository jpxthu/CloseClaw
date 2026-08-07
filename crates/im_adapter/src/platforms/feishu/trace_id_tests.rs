//! Unit tests for trace_id generation in Feishu adapter (Step 1.6).
//!
//! Verifies that `parse_message_event` produces NormalizedMessage with
//! correctly formatted and unique trace_ids.

use super::adapter::FeishuAdapter;
use super::adapter::{FeishuEvent, FeishuHeader, FeishuMessageEvent, FeishuSender, FeishuSenderId};
use std::sync::Arc;

/// Create a test FeishuAdapter (no real HTTP — only sync methods are exercised).
fn make_adapter() -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    FeishuAdapter {
        app_id: "test_app_id".to_string(),
        app_secret: "test_secret".to_string(),
        verification_token: "test_token".to_string(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: super::adapter::FEISHU_API_BASE.to_string(),
    }
}

/// Build a minimal FeishuEvent for a message event.
fn make_event(message_type: &str, content_json: &str) -> FeishuEvent {
    FeishuEvent {
        schema: "2.0".to_string(),
        header: FeishuHeader {
            event_id: "ev_test".to_string(),
            event_type: "im.message.receive_v1".to_string(),
            create_time: "1234567890".to_string(),
            token: "tok".to_string(),
            app_id: "test_app_id".to_string(),
        },
        event: FeishuMessageEvent {
            message_id: None,
            sender: FeishuSender {
                sender_id: FeishuSenderId {
                    open_id: "ou_sender".to_string(),
                },
                sender_type: "user".to_string(),
            },
            content: content_json.to_string(),
            chat_id: "oc_chat".to_string(),
            message_type: message_type.to_string(),
            thread_id: None,
            root_id: None,
            parent_id: None,
        },
    }
}

/// parse_message_event must produce a NormalizedMessage with non-empty trace_id.
#[tokio::test]
async fn test_parse_message_event_has_non_empty_trace_id() {
    let adapter = make_adapter();
    let event = make_event("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert!(!msg.trace_id.is_empty(), "trace_id must not be empty");
}

/// trace_id must follow the format `{platform}-{timestamp_ms}-{uuid_v4}`.
#[tokio::test]
async fn test_parse_message_event_trace_id_format() {
    let adapter = make_adapter();
    let event = make_event("text", &serde_json::json!({"text": "hello"}).to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    let parts: Vec<&str> = msg.trace_id.split('-').collect();
    assert!(
        parts.len() >= 3,
        "trace_id should have at least 3 dash-separated parts"
    );
    assert_eq!(parts[0], "feishu", "platform prefix should be feishu");
    // Part 1 should be a numeric timestamp
    assert!(
        parts[1].parse::<i64>().is_ok(),
        "second part should be a numeric timestamp"
    );
    // Remaining parts form the UUID (with hyphens)
    let uuid_part = parts[2..].join("-");
    assert_eq!(
        uuid_part.len(),
        36,
        "UUID v4 should be 36 chars (8-4-4-4-12)"
    );
}

/// Two calls to parse_message_event must produce different trace_ids (UUID uniqueness).
#[tokio::test]
async fn test_parse_message_event_trace_id_unique() {
    let adapter = make_adapter();
    let event1 = make_event("text", &serde_json::json!({"text": "a"}).to_string());
    let event2 = make_event("text", &serde_json::json!({"text": "b"}).to_string());
    let msg1 = adapter.parse_message_event(event1).await.unwrap().unwrap();
    let msg2 = adapter.parse_message_event(event2).await.unwrap().unwrap();
    assert_ne!(msg1.trace_id, msg2.trace_id, "trace_ids must be unique");
}
