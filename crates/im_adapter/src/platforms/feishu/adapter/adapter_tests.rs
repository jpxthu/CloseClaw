//! Unit tests for Feishu adapter: expand_post_content, parse_message_event
//! (text/post/image/file/audio with graceful degradation), parse_inbound,
//! and send_message/send_card_json receive_id_type verification.

use super::*;
use crate::platforms::feishu::FeishuPlugin;
use crate::plugin::IMPlugin;
use axum::{
    extract::{Path, Query},
    routing::{get, post},
    Json, Router,
};
use closeclaw_common::MessageType;
use closeclaw_config::identity::ConfigIdentityResolver;
use closeclaw_config::identity::IdentityMapping;
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
    make_message_event_with_id(message_type, content_json, None)
}

/// Build a FeishuEvent with an explicit message_id.
fn make_message_event_with_id(
    message_type: &str,
    content_json: &str,
    message_id: Option<&str>,
) -> FeishuEvent {
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
            message_id: message_id.map(String::from),
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
    let mut event_json = serde_json::json!({
        "sender": {
            "sender_id": { "open_id": event.event.sender.sender_id.open_id },
            "sender_type": event.event.sender.sender_type,
        },
        "content": event.event.content,
        "chat_id": event.event.chat_id,
        "message_type": event.event.message_type,
    });
    if let Some(ref mid) = event.event.message_id {
        event_json["message_id"] = serde_json::json!(mid);
    }
    let payload = serde_json::json!({
        "schema": event.schema,
        "header": {
            "event_id": event.header.event_id,
            "event_type": event.header.event_type,
            "create_time": event.header.create_time,
            "token": event.header.token,
            "app_id": event.header.app_id,
        },
        "event": event_json,
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

#[tokio::test]
async fn test_parse_message_event_text_type() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hello"}).to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.message_type, MessageType::Text);
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_parse_message_event_post_type() {
    let adapter = make_test_adapter();
    let content = serde_json::json!({
        "title": "T",
        "content": [[{"tag": "text", "text": "body"}]]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "T\nbody");
    assert_eq!(msg.message_type, MessageType::Other("post".to_string()));
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_parse_message_event_image_discarded() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "image",
        &serde_json::json!({"image_key": "img_xxx"}).to_string(),
        Some("om_msg_001"),
    );
    let result = adapter.parse_message_event(event).await.unwrap();
    assert!(result.is_none(), "image message should be discarded");
}

#[tokio::test]
async fn test_parse_message_event_file_discarded() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "file",
        &serde_json::json!({"file_key": "file_xxx", "file_name": "report.pdf"}).to_string(),
        Some("om_msg_002"),
    );
    let result = adapter.parse_message_event(event).await.unwrap();
    assert!(result.is_none(), "file message should be discarded");
}

#[tokio::test]
async fn test_parse_message_event_audio_discarded() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "audio",
        &serde_json::json!({"file_key": "audio_xxx"}).to_string(),
        Some("om_msg_003"),
    );
    let result = adapter.parse_message_event(event).await.unwrap();
    assert!(result.is_none(), "audio message should be discarded");
}

#[tokio::test]
async fn test_parse_message_event_metadata_account_id() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.account_id, "ou_sender");
}

#[tokio::test]
async fn test_parse_message_event_thread_id_from_root_id() {
    let adapter = make_test_adapter();
    let mut event = make_message_event("text", &serde_json::json!({"text": "hi"}).to_string());
    event.event.root_id = Some("om_root123".to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.thread_id.as_deref(), Some("om_root123"));
}

// ===========================================================================
// Empty text content filtering tests (Step 1.2)
// ===========================================================================

#[tokio::test]
async fn test_parse_text_empty_content_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": ""}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "text with empty content should be discarded"
    );
}

#[tokio::test]
async fn test_parse_text_missing_text_field_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "text with missing text field should be discarded"
    );
}

#[tokio::test]
async fn test_parse_post_empty_expand_returns_none() {
    let adapter = make_test_adapter();
    // Empty content array → expand_post_content returns ""
    let event = make_message_event("post", &serde_json::json!({"content": []}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "post with empty expand should be discarded"
    );
}

#[tokio::test]
async fn test_parse_text_whitespace_only_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "   "}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "text with whitespace-only content should be discarded"
    );
}

#[tokio::test]
async fn test_parse_image_discarded() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "image",
        &serde_json::json!({}).to_string(),
        Some("om_img_empty"),
    );
    let result = adapter.parse_message_event(event).await.unwrap();
    assert!(result.is_none(), "image message should be discarded");
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
async fn test_parse_card_action_forceful_shutdown() {
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
        .parse_card_action(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    let card = result.expect("expected Some(CardActionEvent) for forceful_shutdown");
    assert_eq!(card.platform, "feishu");
    assert_eq!(card.sender_id, "ou_operator");
    assert_eq!(card.action_value, "forceful_shutdown");
    assert_eq!(card.account_id, "ou_operator");
    assert_eq!(card.metadata.get("card_action").unwrap(), "true");
    assert_eq!(card.metadata.get("chat_id").unwrap(), "oc_chat123");
}

#[tokio::test]
async fn test_parse_card_action_unknown_returns_some() {
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
        .parse_card_action(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    // Per design doc: all card actions are returned as CardActionEvent;
    // the Gateway routes them to the tool_result channel.
    let card = result.expect("expected Some(CardActionEvent) for any recognized action");
    assert_eq!(card.action_value, "some_other_action");
}

#[tokio::test]
async fn test_parse_card_action_no_value_returns_none() {
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
        .parse_card_action(&serde_json::to_vec(&payload).unwrap())
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
async fn test_parse_inbound_empty_text_returns_none() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload("text", &serde_json::json!({"text": ""}).to_string());
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_none(),
        "parse_inbound should discard empty text messages"
    );
}

#[tokio::test]
async fn test_parse_inbound_whitespace_only_text_returns_none() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload("text", &serde_json::json!({"text": "   "}).to_string());
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_none(),
        "parse_inbound should discard whitespace-only text messages"
    );
}

#[tokio::test]
async fn test_parse_inbound_text_type() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Text);
    assert_eq!(msg.content, "hi");
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
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Other("post".to_string()));
    assert_eq!(msg.content, "Post\nbody");
}

#[tokio::test]
async fn test_parse_inbound_image_discarded() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload(
        "image",
        &serde_json::json!({"image_key": "img_xxx"}).to_string(),
    );
    let result = plugin.parse_inbound(&payload).await.unwrap();
    assert!(result.is_none(), "image message should be discarded");
}

// ===========================================================================
// Identity mapping tests
// ===========================================================================

#[tokio::test]
async fn test_parse_inbound_with_identity_mapping() {
    let adapter = Arc::new(make_test_adapter());
    let resolver = ConfigIdentityResolver::new(vec![IdentityMapping {
        platform: "feishu".to_string(),
        sender_id: "ou_sender".to_string(),
        account_id: "mapped_user".to_string(),
    }]);
    let plugin = FeishuPlugin::with_identity_resolver(adapter, Some(Arc::new(resolver)));
    let payload = make_webhook_payload("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(msg.account_id, "mapped_user");
    assert_eq!(msg.sender_id, "ou_sender");
}

#[tokio::test]
async fn test_parse_inbound_without_mapping_fallback() {
    let adapter = Arc::new(make_test_adapter());
    // Resolver has a mapping for a different sender, not ou_sender.
    let resolver = ConfigIdentityResolver::new(vec![IdentityMapping {
        platform: "feishu".to_string(),
        sender_id: "ou_other".to_string(),
        account_id: "other_user".to_string(),
    }]);
    let plugin = FeishuPlugin::with_identity_resolver(adapter, Some(Arc::new(resolver)));
    let payload = make_webhook_payload("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    // No matching mapping → fallback to sender_open_id
    assert_eq!(msg.account_id, "ou_sender");
}

#[tokio::test]
async fn test_parse_inbound_no_resolver_fallback() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload("text", &serde_json::json!({"text": "hi"}).to_string());
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    // No resolver at all → fallback to sender_open_id
    assert_eq!(msg.account_id, "ou_sender");
}

// ===========================================================================
// Quote/reference (parent_id) tests
// ===========================================================================

/// Start a mock server that supports GET /im/v1/messages/{message_id}.
/// `messages` maps message_id → (msg_type, body_json_string).
async fn start_quote_mock_server(
    messages: Arc<tokio::sync::Mutex<StdHashMap<String, (String, String)>>>,
) -> String {
    let msgs = Arc::clone(&messages);
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
            "/im/v1/messages/:message_id",
            get(move |Path(message_id): Path<String>| async move {
                let msgs = msgs.lock().await;
                match msgs.get(&message_id) {
                    Some((msg_type, body)) => Json(serde_json::json!({
                        "code": 0,
                        "msg": "ok",
                        "items": [{
                            "msg_type": msg_type,
                            "body": { "content": body }
                        }]
                    })),
                    None => Json(serde_json::json!({
                        "code": 1,
                        "msg": "message not found"
                    })),
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{}", addr)
}

/// Build a FeishuEvent with a parent_id for quote testing.
fn make_message_event_with_parent(
    message_type: &str,
    content_json: &str,
    parent_id: &str,
) -> FeishuEvent {
    let mut event = make_message_event(message_type, content_json);
    event.event.parent_id = Some(parent_id.to_string());
    event
}

/// Build a FeishuEvent with both parent_id and root_id.
fn make_message_event_with_parent_and_root(
    message_type: &str,
    content_json: &str,
    parent_id: &str,
    root_id: &str,
) -> FeishuEvent {
    let mut event = make_message_event(message_type, content_json);
    event.event.parent_id = Some(parent_id.to_string());
    event.event.root_id = Some(root_id.to_string());
    event
}

// --- Test 1: parent_id + API returns text type → content contains blockquote ---

#[tokio::test]
async fn test_quote_text_type_prepends_blockquote() {
    let messages = Arc::new(tokio::sync::Mutex::new(StdHashMap::from([(
        "om_parent1".to_string(),
        (
            "text".to_string(),
            serde_json::json!({"text": "quoted text"}).to_string(),
        ),
    )])));
    let base_url = start_quote_mock_server(messages).await;
    let adapter = make_adapter_with_base(&base_url);
    let event = make_message_event_with_parent(
        "text",
        &serde_json::json!({"text": "reply body"}).to_string(),
        "om_parent1",
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert!(
        msg.content.starts_with("\u{003e} "),
        "should start with blockquote prefix"
    );
    assert!(msg.content.contains("quoted text"));
    assert!(msg.content.contains("reply body"));
}

// --- Test 2: parent_id + API returns post type → content contains expanded blockquote ---

#[tokio::test]
async fn test_quote_post_type_prepends_expanded_blockquote() {
    let post_content = serde_json::json!({
        "title": "Post Title",
        "content": [[{"tag": "text", "text": "post body"}]]
    });
    let messages = Arc::new(tokio::sync::Mutex::new(StdHashMap::from([(
        "om_parent2".to_string(),
        ("post".to_string(), post_content.to_string()),
    )])));
    let base_url = start_quote_mock_server(messages).await;
    let adapter = make_adapter_with_base(&base_url);
    let event = make_message_event_with_parent(
        "text",
        &serde_json::json!({"text": "my reply"}).to_string(),
        "om_parent2",
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert!(msg.content.contains("\u{003e} Post Title"));
    assert!(msg.content.contains("\u{003e} post body"));
    assert!(msg.content.contains("my reply"));
}

// --- Test 3: quote content > 500 chars → truncated with "..." ---

#[tokio::test]
async fn test_quote_truncates_at_500_chars() {
    let long_text = "a".repeat(600);
    let messages = Arc::new(tokio::sync::Mutex::new(StdHashMap::from([(
        "om_parent3".to_string(),
        (
            "text".to_string(),
            serde_json::json!({"text": &long_text}).to_string(),
        ),
    )])));
    let base_url = start_quote_mock_server(messages).await;
    let adapter = make_adapter_with_base(&base_url);
    let event = make_message_event_with_parent(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        "om_parent3",
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    // The blockquote line should be "> " + 500 chars + "..."
    let first_line = msg.content.lines().next().unwrap();
    assert!(first_line.starts_with("> "));
    let quoted_part = &first_line[2..]; // strip "> "
    assert!(quoted_part.ends_with("..."));
    assert_eq!(quoted_part.len(), 503); // 500 + "..."
}

// --- Test 4: quote content exactly 500 chars → no "..." ---

#[tokio::test]
async fn test_quote_exactly_500_chars_no_truncation() {
    let exact_text = "b".repeat(500);
    let messages = Arc::new(tokio::sync::Mutex::new(StdHashMap::from([(
        "om_parent4".to_string(),
        (
            "text".to_string(),
            serde_json::json!({"text": &exact_text}).to_string(),
        ),
    )])));
    let base_url = start_quote_mock_server(messages).await;
    let adapter = make_adapter_with_base(&base_url);
    let event = make_message_event_with_parent(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        "om_parent4",
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    let first_line = msg.content.lines().next().unwrap();
    let quoted_part = &first_line[2..]; // strip "> "
    assert_eq!(quoted_part, exact_text);
    assert!(!quoted_part.ends_with("..."));
}

// --- Test 5: parent_id exists but API fails → no blockquote ---

#[tokio::test]
async fn test_quote_api_failure_no_blockquote() {
    let messages: Arc<tokio::sync::Mutex<StdHashMap<String, (String, String)>>> =
        Arc::new(tokio::sync::Mutex::new(StdHashMap::new()));
    let base_url = start_quote_mock_server(messages).await;
    let adapter = make_adapter_with_base(&base_url);
    let event = make_message_event_with_parent(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        "om_nonexistent",
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "reply");
}

// --- Test 6: parent_id exists but message type is image → no blockquote ---

#[tokio::test]
async fn test_quote_image_type_no_blockquote() {
    let messages = Arc::new(tokio::sync::Mutex::new(StdHashMap::from([(
        "om_parent6".to_string(),
        (
            "image".to_string(),
            serde_json::json!({"image_key": "img_xxx"}).to_string(),
        ),
    )])));
    let base_url = start_quote_mock_server(messages).await;
    let adapter = make_adapter_with_base(&base_url);
    let event = make_message_event_with_parent(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        "om_parent6",
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "reply");
}

// --- Test 7: no parent_id → behavior unchanged ---

#[tokio::test]
async fn test_no_parent_id_unchanged_behavior() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hello"}).to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "hello");
    assert!(!msg.content.contains("> "));
}

// --- Test 8: parent_id + root_id → thread_id uses root_id, quote still works ---

#[tokio::test]
async fn test_quote_with_root_id_thread_uses_root_id() {
    let messages = Arc::new(tokio::sync::Mutex::new(StdHashMap::from([(
        "om_parent8".to_string(),
        (
            "text".to_string(),
            serde_json::json!({"text": "quoted"}).to_string(),
        ),
    )])));
    let base_url = start_quote_mock_server(messages).await;
    let adapter = make_adapter_with_base(&base_url);
    let event = make_message_event_with_parent_and_root(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        "om_parent8",
        "om_root99",
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    // thread_id should be root_id, not parent_id
    assert_eq!(msg.thread_id.as_deref(), Some("om_root99"));
    // quote content should still be present
    assert!(msg.content.contains("\u{003e} quoted"));
    assert!(msg.content.contains("reply"));
}

// --- Tests for truncate_to_500 UTF-8 handling ---
#[test]
fn test_truncate_to_500_ascii_within_limit() {
    assert_eq!(truncate_to_500(&"a".repeat(500)), "a".repeat(500));
}
#[test]
fn test_truncate_to_500_ascii_exceeds_limit() {
    let result = truncate_to_500(&"a".repeat(600));
    assert!(result.ends_with("..."));
    assert_eq!(result.len(), 503);
    assert_eq!(result.chars().count(), 503);
}
#[test]
fn test_truncate_to_500_chinese_within_limit() {
    let chinese = "中".repeat(500);
    assert_eq!(truncate_to_500(&chinese), chinese);
}
#[test]
fn test_truncate_to_500_chinese_exceeds_limit() {
    let result = truncate_to_500(&"中".repeat(600));
    assert!(result.ends_with("..."));
    assert_eq!(result.len(), 1503);
    assert_eq!(result.chars().count(), 503);
}
#[test]
fn test_truncate_to_500_mixed_text() {
    let mixed = format!("{}{}", "中".repeat(400), "a".repeat(200));
    let result = truncate_to_500(&mixed);
    assert!(result.ends_with("..."));
    assert_eq!(result.len(), 1303);
    assert_eq!(result.chars().count(), 503);
}
#[test]
fn test_truncate_to_500_empty_string() {
    assert_eq!(truncate_to_500(""), "");
}
