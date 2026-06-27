//! Unit tests for Feishu adapter: expand_post_content, parse_message_event,
//! parse_inbound (message_type propagation), and send_message/send_card_json
//! receive_id_type verification.

use super::*;
use crate::platforms::feishu::FeishuPlugin;
use crate::plugin::IMPlugin;
use axum::{extract::Query, routing::post, Json, Router};
use std::collections::HashMap as StdHashMap;
use tokio::net::TcpListener;

/// Create a test FeishuAdapter (no real HTTP — only sync methods are exercised).
fn make_test_adapter() -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    FeishuAdapter {
        app_id: "test_app_id".to_string(),
        app_secret: "test_secret".to_string(),
        verification_token: "test_token".to_string(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: FEISHU_API_BASE.to_string(),
    }
}

/// Create a FeishuAdapter pointing at a mock server.
fn make_adapter_with_base(base_url: &str) -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    FeishuAdapter {
        app_id: "test_app_id".to_string(),
        app_secret: "test_secret".to_string(),
        verification_token: "test_token".to_string(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: base_url.to_string(),
    }
}

/// Start a minimal mock Feishu API server, return its base URL.
/// Tracks whether `receive_id_type=chat_id` was sent on `/im/v1/messages`.
async fn start_mock_server(received_id_type: Arc<tokio::sync::Mutex<Option<String>>>) -> String {
    let app = Router::new()
        .route(
            "/auth/v3/tenant_access_token/internal",
            post(|Json(_body): Json<serde_json::Value>| async move {
                Json(serde_json::json!({
                    "code": 0,
                    "msg": "ok",
                    "tenant_access_token": "mock_token"
                }))
            }),
        )
        .route(
            "/im/v1/messages",
            post(
                move |Query(params): Query<StdHashMap<String, String>>,
                      Json(_body): Json<serde_json::Value>| async move {
                    let rid = params.get("receive_id_type").cloned();
                    *received_id_type.lock().await = rid;
                    Json(serde_json::json!({"code": 0, "msg": "ok"}))
                },
            ),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{}", addr)
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
fn test_expand_post_img_tag_with_text() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "img", "text": "alt text"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "[图片]");
}

#[test]
fn test_expand_post_media_tag() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "media"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "[视频]");
}

#[test]
fn test_expand_post_file_tag() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "file"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "[文件]");
}

#[test]
fn test_expand_post_unknown_tag_with_text() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "some_unknown", "text": "fallback text"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "fallback text");
}

#[test]
fn test_expand_post_unknown_tag_without_text() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "unknown"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "[未知消息]");
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

#[test]
fn test_expand_post_title_with_mixed_elements() {
    let content = serde_json::json!({
        "title": "Mixed Post",
        "content": [
            [{"tag": "text", "text": "Hello "}, {"tag": "at", "name": "Bob"}],
            [{"tag": "img"}],
            [{"tag": "text", "text": "Caption"}],
            [{"tag": "file"}],
            [{"tag": "media"}],
            [{"tag": "a", "text": "link", "href": "https://x.com"}]
        ]
    });
    assert_eq!(
        expand_post_content(&content),
        "Mixed Post\nHello @Bob\n[图片]\nCaption\n[文件]\n[视频]\nlink"
    );
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
    assert_eq!(msg.message_type, "text");
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
    assert_eq!(msg.message_type, "post");
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
fn test_parse_message_event_audio_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event(
        "audio",
        &serde_json::json!({"file_key": "audio_xxx"}).to_string(),
    );
    let result = adapter.parse_message_event(event).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_parse_message_event_metadata_account_id() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = adapter.parse_message_event(event).unwrap().unwrap();
    assert_eq!(msg.account_id.as_deref(), Some("ou_sender"));
}

#[test]
fn test_parse_message_event_thread_id_from_root_id() {
    let adapter = make_test_adapter();
    let mut event = make_message_event("text", &serde_json::json!({"text": "hi"}).to_string());
    event.event.root_id = Some("om_root123".to_string());
    let msg = adapter.parse_message_event(event).unwrap().unwrap();
    assert_eq!(msg.thread_id.as_deref(), Some("om_root123"));
}

// ===========================================================================
// send_message / send_card_json receive_id_type tests
// ===========================================================================

#[tokio::test]
async fn test_send_message_uses_chat_id_receive_id_type() {
    let received = Arc::new(tokio::sync::Mutex::new(None));
    let base_url = start_mock_server(Arc::clone(&received)).await;

    let adapter = make_adapter_with_base(&base_url);
    let msg = Message {
        id: "1".into(),
        from: "a".into(),
        to: "oc_target_chat".into(),
        content: "hello".into(),
        channel: "feishu".into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
    };

    adapter.send_message(&msg, None).await.unwrap();
    assert_eq!(received.lock().await.as_deref(), Some("chat_id"));
}

#[tokio::test]
async fn test_send_card_json_uses_chat_id_receive_id_type() {
    let received = Arc::new(tokio::sync::Mutex::new(None));
    let base_url = start_mock_server(Arc::clone(&received)).await;

    let adapter = make_adapter_with_base(&base_url);
    let card = serde_json::json!({"header": {}, "elements": []}).to_string();

    adapter
        .send_card_json("oc_target_chat", &card, None)
        .await
        .unwrap();
    assert_eq!(received.lock().await.as_deref(), Some("chat_id"));
}

#[tokio::test]
async fn test_send_message_and_card_use_consistent_receive_id_type() {
    let received = Arc::new(tokio::sync::Mutex::new(None));
    let base_url = start_mock_server(Arc::clone(&received)).await;

    let adapter = make_adapter_with_base(&base_url);

    // Send text message
    let msg = Message {
        id: "1".into(),
        from: "a".into(),
        to: "oc_chat".into(),
        content: "hi".into(),
        channel: "feishu".into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
    };
    adapter.send_message(&msg, None).await.unwrap();
    assert_eq!(received.lock().await.as_deref(), Some("chat_id"));

    // Send card message
    *received.lock().await = None;
    let card = serde_json::json!({"header": {}, "elements": []}).to_string();
    adapter
        .send_card_json("oc_chat", &card, None)
        .await
        .unwrap();
    assert_eq!(received.lock().await.as_deref(), Some("chat_id"));
}

// ===========================================================================
// handle_webhook card action tests
// ===========================================================================

#[tokio::test]
async fn test_handle_webhook_card_action_forceful_shutdown() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "evt_card_1",
            "event_type": "card.action.trigger",
            "create_time": "1234567890",
            "token": "tok",
            "app_id": "test_app_id"
        },
        "operator": {
            "open_id": "ou_operator"
        },
        "token": "card_token",
        "action": {
            "value": {"action": "forceful_shutdown", "chat_id": "oc_chat123"},
            "tag": "button"
        }
    });
    let result = adapter
        .handle_webhook(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    let msg = result.expect("expected Some(NormalizedMessage) for forceful_shutdown");
    assert_eq!(msg.sender_id, "ou_operator");
    assert_eq!(msg.content, "/__card_action:forceful_shutdown");
    assert_eq!(msg.message_type, "text");
    assert_eq!(msg.card_action, Some(true));
    assert_eq!(msg.platform, "feishu");
    assert_eq!(msg.peer_id, "oc_chat123");
    assert_eq!(msg.account_id.as_deref(), Some("ou_operator"));
}

#[tokio::test]
async fn test_handle_webhook_card_action_unknown_returns_none() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "evt_card_2",
            "event_type": "card.action.trigger",
            "create_time": "1234567890",
            "token": "tok",
            "app_id": "test_app_id"
        },
        "operator": {
            "open_id": "ou_operator"
        },
        "token": "card_token",
        "action": {
            "value": {"action": "some_other_action"},
            "tag": "button"
        }
    });
    let result = adapter
        .handle_webhook(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    assert!(result.is_none(), "unknown card action should return None");
}

#[tokio::test]
async fn test_handle_webhook_card_action_no_value_returns_none() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "evt_card_3",
            "event_type": "card.action.trigger",
            "create_time": "1234567890",
            "token": "tok",
            "app_id": "test_app_id"
        },
        "operator": {
            "open_id": "ou_operator"
        },
        "token": "card_token",
        "action": {
            "value": null,
            "tag": "button"
        }
    });
    let result = adapter
        .handle_webhook(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "card action with null value should return None"
    );
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
