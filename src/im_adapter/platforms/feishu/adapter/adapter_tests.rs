//! Unit tests for Feishu adapter: expand_post_content, parse_message_event,
//! and parse_inbound (message_type propagation).

use super::*;
use crate::im_adapter::platforms::feishu::FeishuPlugin;
use crate::im_adapter::plugin::IMPlugin;

/// Create a test FeishuAdapter (no real HTTP — only sync methods are exercised).
fn make_test_adapter() -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    FeishuAdapter {
        app_id: "test_app_id".to_string(),
        app_secret: "test_secret".to_string(),
        verification_token: "test_token".to_string(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
    }
}

/// Build a minimal FeishuEvent for a message event.
fn make_message_event(message_type: &str, content_json: &str) -> FeishuEvent {
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

/// Build a webhook payload JSON from a message event.
fn make_webhook_payload(message_type: &str, content_json: &str) -> Vec<u8> {
    let event = make_message_event(message_type, content_json);
    let payload = serde_json::json!({
        "schema": event.schema,
        "header": {
            "event_id": event.header.event_id,
            "event_type": event.header.event_type,
            "create_time": event.header.create_time,
            "token": event.header.token,
            "app_id": event.header.app_id,
        },
        "event": {
            "sender": {
                "sender_id": { "open_id": event.event.sender.sender_id.open_id },
                "sender_type": event.event.sender.sender_type,
            },
            "content": event.event.content,
            "chat_id": event.event.chat_id,
            "message_type": event.event.message_type,
        },
    });
    serde_json::to_vec(&payload).unwrap()
}

// ===========================================================================
// expand_post_content tests
// ===========================================================================

#[test]
fn test_expand_post_pure_text() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "text", "text": "hello "},
            {"tag": "text", "text": "world"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "hello world");
}

#[test]
fn test_expand_post_with_link() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "text", "text": "visit "},
            {"tag": "a", "text": "click here", "href": "https://example.com"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "visit click here");
}

#[test]
fn test_expand_post_with_at() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "at", "name": "Alice", "user_id": "ou_123"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "@Alice");
}

#[test]
fn test_expand_post_at_without_name() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "at", "user_id": "ou_456"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "@ou_456");
}

#[test]
fn test_expand_post_with_title() {
    let content = serde_json::json!({
        "title": "My Title",
        "content": [[
            {"tag": "text", "text": "body"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "My Title\nbody");
}

#[test]
fn test_expand_post_unknown_tag_with_text() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "img", "text": "alt text"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "alt text");
}

#[test]
fn test_expand_post_unknown_tag_without_text() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "unknown"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "");
}

#[test]
fn test_expand_post_empty_content() {
    let content = serde_json::json!({"content": []});
    assert_eq!(expand_post_content(&content), "");
}

#[test]
fn test_expand_post_no_content_key() {
    let content = serde_json::json!({});
    assert_eq!(expand_post_content(&content), "");
}

#[test]
fn test_expand_post_multiple_rows() {
    let content = serde_json::json!({
        "content": [
            [{"tag": "text", "text": "line1"}],
            [{"tag": "text", "text": "line2"}]
        ]
    });
    assert_eq!(expand_post_content(&content), "line1\nline2");
}

// ===========================================================================
// parse_message_event tests
// ===========================================================================

#[test]
fn test_parse_message_event_text_type() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hello"}).to_string());
    let msg = adapter.parse_message_event(event).unwrap().unwrap();
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.metadata.get("message_type").unwrap(), "text");
}

#[test]
fn test_parse_message_event_post_type() {
    let adapter = make_test_adapter();
    let content = serde_json::json!({
        "title": "T",
        "content": [[{"tag": "text", "text": "body"}]]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).unwrap().unwrap();
    assert_eq!(msg.content, "T\nbody");
    assert_eq!(msg.metadata.get("message_type").unwrap(), "post");
}

#[test]
fn test_parse_message_event_image_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event(
        "image",
        &serde_json::json!({"image_key": "img_xxx"}).to_string(),
    );
    let result = adapter.parse_message_event(event).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_parse_message_event_file_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event(
        "file",
        &serde_json::json!({"file_key": "file_xxx"}).to_string(),
    );
    let result = adapter.parse_message_event(event).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_parse_message_event_metadata_account_id() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = adapter.parse_message_event(event).unwrap().unwrap();
    assert_eq!(msg.metadata.get("account_id").unwrap(), "test_app_id");
}

#[test]
fn test_parse_message_event_thread_id_from_root_id() {
    let adapter = make_test_adapter();
    let mut event = make_message_event("text", &serde_json::json!({"text": "hi"}).to_string());
    event.event.root_id = Some("om_root123".to_string());
    let msg = adapter.parse_message_event(event).unwrap().unwrap();
    assert_eq!(msg.metadata.get("thread_id").unwrap(), "om_root123");
}

// ===========================================================================
// parse_inbound tests (message_type propagation)
// ===========================================================================

#[tokio::test]
async fn test_parse_inbound_text_type() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload("text", &serde_json::json!({"text": "hi"}).to_string());
    let normalized = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(normalized.message_type, "text");
    assert_eq!(normalized.content, "hi");
}

#[tokio::test]
async fn test_parse_inbound_post_type() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let content = serde_json::json!({
        "title": "Post",
        "content": [[{"tag": "text", "text": "body"}]]
    });
    let payload = make_webhook_payload("post", &content.to_string());
    let normalized = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(normalized.message_type, "post");
    assert_eq!(normalized.content, "Post\nbody");
}

#[tokio::test]
async fn test_parse_inbound_image_returns_none() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload(
        "image",
        &serde_json::json!({"image_key": "img_xxx"}).to_string(),
    );
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(result.is_none());
}
