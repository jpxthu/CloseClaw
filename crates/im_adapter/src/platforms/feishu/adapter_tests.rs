//! Unit tests for Feishu adapter: expand_post_content, parse_message_event
//! (text/post/image/file/audio with graceful degradation), parse_inbound,
//! parse_card_action, identity mapping, and quote/reference handling.
use super::*;
use crate::media_store::MediaStore;
use crate::platforms::feishu::FeishuPlugin;
use crate::plugin::IMPlugin;
use closeclaw_common::MessageType;
use closeclaw_config::identity::ConfigIdentityResolver;
use closeclaw_config::identity::IdentityMapping;
use tempfile::TempDir;

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

/// Create a test FeishuAdapter (no real HTTP — only sync methods are exercised).
fn make_test_adapter() -> FeishuAdapter {
    FeishuAdapter::new("test_profile".to_string(), make_test_media_store())
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
            chat_type: None,
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
    assert_eq!(expand_post_content(&content), "visit ");
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
        "Mixed Post\nHello @Bob\n[图片]\nCaption\n[文件]\n[视频]\n"
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
    assert_eq!(msg.message_type, MessageType::Post);
    assert!(msg.media_refs.is_empty());
}
#[tokio::test]
async fn test_parse_message_event_image_type() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "image",
        &serde_json::json!({"image_key": "img_xxx"}).to_string(),
        Some("om_msg_001"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Image);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert!(msg.content.is_empty());
}
#[tokio::test]
async fn test_parse_message_event_file_type() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "file",
        &serde_json::json!({"file_key": "file_xxx", "file_name": "report.pdf"}).to_string(),
        Some("om_msg_002"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::File);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert!(msg.content.is_empty());
}
#[tokio::test]
async fn test_parse_message_event_audio_type() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "audio",
        &serde_json::json!({"file_key": "audio_xxx"}).to_string(),
        Some("om_msg_003"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Audio);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert!(msg.content.is_empty());
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
// card.action.trigger deferred tests (Step 1.4)
// ===========================================================================

/// card.action.trigger is deferred per design doc: parse_card_action
/// returns Ok(None) and logs debug info. The Gateway discards None.
#[tokio::test]
async fn test_parse_card_action_deferred_returns_none() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "evt_card_deferred_1",
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
    assert!(
        result.is_none(),
        "card.action.trigger should return None (deferred per design doc)"
    );
}

/// card.action.trigger with CLI-format fixture payload also returns None.
#[tokio::test]
async fn test_parse_card_action_cli_fixture_deferred() {
    let adapter = make_test_adapter();
    // CLI format: top-level type/event_id, operator_id, action_value
    let payload = serde_json::json!({
        "type": "card.action.trigger",
        "event_id": "cli_card_evt_1",
        "timestamp": "1787884959729992",
        "operator_id": "ou_op_cli",
        "message_id": "om_test",
        "chat_id": "oc_chat_cli",
        "host": "im_message",
        "token": "card_token_cli",
        "action_tag": "button",
        "action_value": "{\"action\":\"approve\",\"task\":\"t1\"}",
        "checked": false
    });
    let result = adapter
        .parse_card_action(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "CLI card.action.trigger should return None (deferred per design doc)"
    );
}

/// Duplicate card.action.trigger event_id is rejected via dedup.
#[tokio::test]
async fn test_parse_card_action_dedup_duplicate_rejected() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "evt_card_dedup_1",
            "event_type": "card.action.trigger",
            "create_time": "0",
            "token": "t",
            "app_id": "a"
        },
        "operator": {"open_id": "ou_op"},
        "token": "tok",
        "action": {"value": {"action": "btn"}}
    });
    // First call: deferred, records event_id in dedup.
    let _ = adapter
        .parse_card_action(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    // Second call with same event_id: dedup rejects it.
    let result = adapter
        .parse_card_action(&serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "duplicate card.action.trigger event_id should be rejected"
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
    assert_eq!(msg.message_type, MessageType::Post);
    assert_eq!(msg.content, "Post\nbody");
}
#[tokio::test]
async fn test_parse_inbound_image_type() {
    let adapter = Arc::new(make_test_adapter());
    let plugin = FeishuPlugin::new(adapter);
    let payload = make_webhook_payload(
        "image",
        &serde_json::json!({"image_key": "img_xxx"}).to_string(),
    );
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Image);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert!(msg.content.is_empty());
}
// ===========================================================================
// Identity mapping tests
// ===========================================================================
#[tokio::test]
async fn test_parse_inbound_with_identity_mapping() {
    let adapter = Arc::new(make_test_adapter());
    let resolver = ConfigIdentityResolver::new(vec![IdentityMapping {
        platform: "feishu".to_string(),
        bot_app_id: "test_app_id".to_string(),
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
        bot_app_id: "test_app_id".to_string(),
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
// Quote/reference (parent_id) tests

/// Create a FeishuAdapter with a mock lark-cli command.
fn make_adapter_with_mock_cli(mock_cli_path: &str) -> FeishuAdapter {
    let mut adapter = make_test_adapter();
    adapter.cli_command = mock_cli_path.to_string();
    adapter
}

/// Create a mock lark-cli script that handles multiple message IDs.
/// `responses` maps message_id → JSON response string.
fn create_mock_cli_with_messages(
    tmp: &TempDir,
    responses: &std::collections::HashMap<String, String>,
) -> String {
    let script_path = tmp.path().join("mock_lark_cli_msgs");
    let mut script = String::from("#!/bin/sh\n");
    script.push_str(&format!("MSG_ID=\"{}\"\n", ""));
    // Parse --message-id from args
    script.push_str("while [ $# -gt 0 ]; do\n");
    script.push_str("  case \"$1\" in\n");
    script.push_str("    --message-id) MSG_ID=\"$2\"; shift 2;;\n");
    script.push_str("    *) shift;;\n");
    script.push_str("  esac\n");
    script.push_str("done\n");
    for (msg_id, resp) in responses {
        script.push_str(&format!(
            "if [ \"$MSG_ID\" = \"{}\" ]; then echo '{}'; exit 0; fi\n",
            msg_id, resp
        ));
    }
    script.push_str("echo '{\"code\":1,\"msg\":\"not found\"}'\n");
    std::fs::write(&script_path, &script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    }
    script_path.to_str().unwrap().to_string()
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

// --- Test 1: parent_id + CLI returns text type → content contains blockquote ---

#[tokio::test]
async fn test_quote_text_type_prepends_blockquote() {
    let tmp = TempDir::new().unwrap();
    let mut msgs = std::collections::HashMap::new();
    msgs.insert(
        "om_parent1".to_string(),
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "items": [{"msg_type": "text", "body": {"content": serde_json::json!({"text": "quoted text"}).to_string()}}]
        }).to_string(),
    );
    let cli = create_mock_cli_with_messages(&tmp, &msgs);
    let adapter = make_adapter_with_mock_cli(&cli);
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

// --- Test 2: parent_id + CLI returns post type → content contains expanded blockquote ---

#[tokio::test]
async fn test_quote_post_type_prepends_expanded_blockquote() {
    let tmp = TempDir::new().unwrap();
    let post_content = serde_json::json!({
        "title": "Post Title",
        "content": [[{"tag": "text", "text": "post body"}]]
    });
    let mut msgs = std::collections::HashMap::new();
    msgs.insert(
        "om_parent2".to_string(),
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "items": [{"msg_type": "post", "body": {"content": post_content.to_string()}}]
        })
        .to_string(),
    );
    let cli = create_mock_cli_with_messages(&tmp, &msgs);
    let adapter = make_adapter_with_mock_cli(&cli);
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
    let tmp = TempDir::new().unwrap();
    let long_text = "a".repeat(600);
    let mut msgs = std::collections::HashMap::new();
    msgs.insert(
        "om_parent3".to_string(),
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "items": [{"msg_type": "text", "body": {"content": serde_json::json!({"text": &long_text}).to_string()}}]
        }).to_string(),
    );
    let cli = create_mock_cli_with_messages(&tmp, &msgs);
    let adapter = make_adapter_with_mock_cli(&cli);
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
    let tmp = TempDir::new().unwrap();
    let exact_text = "b".repeat(500);
    let mut msgs = std::collections::HashMap::new();
    msgs.insert(
        "om_parent4".to_string(),
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "items": [{"msg_type": "text", "body": {"content": serde_json::json!({"text": &exact_text}).to_string()}}]
        }).to_string(),
    );
    let cli = create_mock_cli_with_messages(&tmp, &msgs);
    let adapter = make_adapter_with_mock_cli(&cli);
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

// --- Test 5: parent_id exists but CLI fails → no blockquote ---

#[tokio::test]
async fn test_quote_api_failure_no_blockquote() {
    let tmp = TempDir::new().unwrap();
    let msgs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let cli = create_mock_cli_with_messages(&tmp, &msgs);
    let adapter = make_adapter_with_mock_cli(&cli);
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
    let tmp = TempDir::new().unwrap();
    let mut msgs = std::collections::HashMap::new();
    msgs.insert(
        "om_parent6".to_string(),
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "items": [{"msg_type": "image", "body": {"content": serde_json::json!({"image_key": "img_xxx"}).to_string()}}]
        }).to_string(),
    );
    let cli = create_mock_cli_with_messages(&tmp, &msgs);
    let adapter = make_adapter_with_mock_cli(&cli);
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
    let tmp = TempDir::new().unwrap();
    let mut msgs = std::collections::HashMap::new();
    msgs.insert(
        "om_parent8".to_string(),
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "items": [{"msg_type": "text", "body": {"content": serde_json::json!({"text": "quoted"}).to_string()}}]
        }).to_string(),
    );
    let cli = create_mock_cli_with_messages(&tmp, &msgs);
    let adapter = make_adapter_with_mock_cli(&cli);
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
#[test]
fn test_extract_unknown_type_err() {
    assert!(FeishuAdapter::extract_message_content("unsupported", &serde_json::json!({})).is_err());
}
#[tokio::test]
async fn test_parse_unknown_type_none() {
    let a = make_test_adapter();
    let e = make_message_event("unsupported_type", &serde_json::json!({}).to_string());
    assert!(a.parse_message_event(e).await.unwrap().is_none());
}

// ===========================================================================
// Session anchor construction tests (Step 1.1)
// ===========================================================================

/// Top-level message: peer_id = sender_open_id|message_id, reply_ref = message_id
#[tokio::test]
async fn test_anchor_top_level_message() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "hi"}).to_string(),
        Some("om_msg_123"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.peer_id, "ou_sender|om_msg_123");
    assert_eq!(msg.reply_ref.as_deref(), Some("om_msg_123"));
    assert_eq!(msg.message_id, "om_msg_123");
}

/// Topic reply with thread_id: peer_id = sender_open_id|thread_id, reply_ref = root_id
#[tokio::test]
async fn test_anchor_topic_reply_with_thread_id() {
    let adapter = make_test_adapter();
    let mut event = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        Some("om_msg_456"),
    );
    event.event.thread_id = Some("om_thread_789".to_string());
    event.event.root_id = Some("om_root_100".to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.peer_id, "ou_sender|om_thread_789");
    assert_eq!(msg.reply_ref.as_deref(), Some("om_root_100"));
    assert_eq!(msg.thread_id.as_deref(), Some("om_thread_789"));
}

/// Topic reply with root_id only (no thread_id): peer_id = sender_open_id|root_id, reply_ref = root_id
#[tokio::test]
async fn test_anchor_topic_reply_root_id_only() {
    let adapter = make_test_adapter();
    let mut event = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        Some("om_msg_200"),
    );
    event.event.root_id = Some("om_root_300".to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.peer_id, "ou_sender|om_root_300");
    assert_eq!(msg.reply_ref.as_deref(), Some("om_root_300"));
    assert_eq!(msg.thread_id.as_deref(), Some("om_root_300"));
}

/// Topic reply with parent_id only: peer_id = sender_open_id|parent_id, reply_ref = parent_id
#[tokio::test]
async fn test_anchor_topic_reply_parent_id_only() {
    let adapter = make_test_adapter();
    let mut event = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "reply"}).to_string(),
        Some("om_msg_400"),
    );
    event.event.parent_id = Some("om_parent_500".to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.peer_id, "ou_sender|om_parent_500");
    assert_eq!(msg.reply_ref.as_deref(), Some("om_parent_500"));
    assert_eq!(msg.thread_id.as_deref(), Some("om_parent_500"));
}

/// Missing message_id: peer_id = sender_open_id|"" , reply_ref = Some("")
#[tokio::test]
async fn test_anchor_missing_message_id() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hi"}).to_string());
    // message_id is None → defaults to empty string
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.peer_id, "ou_sender|");
    assert_eq!(msg.reply_ref.as_deref(), Some(""));
    assert_eq!(msg.message_id, "");
}

/// account_id is unchanged when peer_id is composite
#[tokio::test]
async fn test_anchor_account_id_unchanged() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "hi"}).to_string(),
        Some("om_msg_999"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.account_id, "ou_sender");
    assert_eq!(msg.peer_id, "ou_sender|om_msg_999");
}

/// thread_id fallback chain regression: thread_id > root_id > parent_id
#[tokio::test]
async fn test_anchor_thread_id_fallback_chain() {
    let adapter = make_test_adapter();

    // Case 1: explicit thread_id wins
    let mut e1 = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "r"}).to_string(),
        Some("m1"),
    );
    e1.event.thread_id = Some("t1".to_string());
    e1.event.root_id = Some("r1".to_string());
    e1.event.parent_id = Some("p1".to_string());
    let msg1 = adapter.parse_message_event(e1).await.unwrap().unwrap();
    assert_eq!(msg1.thread_id.as_deref(), Some("t1"));
    assert_eq!(msg1.peer_id, "ou_sender|t1");

    // Case 2: root_id fallback when thread_id absent
    let mut e2 = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "r"}).to_string(),
        Some("m2"),
    );
    e2.event.root_id = Some("r2".to_string());
    let msg2 = adapter.parse_message_event(e2).await.unwrap().unwrap();
    assert_eq!(msg2.thread_id.as_deref(), Some("r2"));
    assert_eq!(msg2.peer_id, "ou_sender|r2");

    // Case 3: parent_id fallback when thread_id and root_id absent
    let mut e3 = make_message_event_with_id(
        "text",
        &serde_json::json!({"text": "r"}).to_string(),
        Some("m3"),
    );
    e3.event.parent_id = Some("p3".to_string());
    let msg3 = adapter.parse_message_event(e3).await.unwrap().unwrap();
    assert_eq!(msg3.thread_id.as_deref(), Some("p3"));
    assert_eq!(msg3.peer_id, "ou_sender|p3");
}
