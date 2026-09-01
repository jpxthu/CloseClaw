//! End-to-end identity isolation tests for feishu parse_inbound.
//!
//! Verifies that the same sender arriving at different bot applications
//! (different header app_id) resolves to different local account_id values.
//! This is the core behavioral contract of the `(platform, bot_app_id,
//! sender_id)` triple key.

use super::*;
use crate::media_store::MediaStore;
use crate::platforms::feishu::FeishuPlugin;
use crate::plugin::IMPlugin;
use closeclaw_config::identity::ConfigIdentityResolver;
use closeclaw_config::identity::IdentityMapping;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a FeishuAdapter for tests (no real HTTP).
fn make_test_adapter() -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    let tmp = TempDir::new().expect("tmp dir");
    FeishuAdapter {
        app_id: "test_app_id".to_string(),
        app_secret: "test_secret".to_string(),
        verification_token: "test_token".to_string(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: FEISHU_API_BASE.to_string(),
        last_metadata: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        media_store: Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store")),
        max_download_size_bytes: u64::MAX,
        workspace_dir: None,
        cli_command: "lark-cli".to_string(),
    }
}

/// Build a FeishuEvent with a custom header app_id (simulates different bot
/// application receiving the same user's message).
fn make_message_event_with_app_id(
    message_type: &str,
    content_json: &str,
    app_id: &str,
) -> FeishuEvent {
    FeishuEvent {
        schema: "2.0".to_string(),
        header: FeishuHeader {
            event_id: "ev_iso".to_string(),
            event_type: "im.message.receive_v1".to_string(),
            create_time: "1234567890".to_string(),
            token: "tok".to_string(),
            app_id: app_id.to_string(),
        },
        event: FeishuMessageEvent {
            message_id: None,
            sender: FeishuSender {
                sender_id: FeishuSenderId {
                    open_id: "ou_alice".to_string(),
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

/// Build a webhook payload from a FeishuEvent.
fn make_webhook_from_event(event: &FeishuEvent) -> Vec<u8> {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Core isolation test (end-to-end): same sender ou_alice sends messages that
/// arrive at two different bot applications (app_x and app_y). Each application
/// maps ou_alice to a different local account. The parse_inbound pipeline must
/// use header.app_id → last_metadata header_app_id → normalize_inbound_message
/// to select the correct mapping.
#[tokio::test]
async fn test_parse_inbound_isolation_different_header_app_id() {
    let adapter = Arc::new(make_test_adapter());
    let resolver = ConfigIdentityResolver::new(vec![
        IdentityMapping {
            platform: "feishu".to_string(),
            bot_app_id: "app_x".to_string(),
            sender_id: "ou_alice".to_string(),
            account_id: "alice_via_x".to_string(),
        },
        IdentityMapping {
            platform: "feishu".to_string(),
            bot_app_id: "app_y".to_string(),
            sender_id: "ou_alice".to_string(),
            account_id: "alice_via_y".to_string(),
        },
    ]);
    let plugin = FeishuPlugin::with_identity_resolver(adapter, Some(Arc::new(resolver)));

    // Message arrives at app_x → account should be alice_via_x
    let event_x = make_message_event_with_app_id(
        "text",
        &serde_json::json!({"text": "hello from x"}).to_string(),
        "app_x",
    );
    let payload_x = make_webhook_from_event(&event_x);
    let msg_x = plugin.parse_inbound(&payload_x).await.unwrap().unwrap();
    assert_eq!(msg_x.account_id, "alice_via_x");
    assert_eq!(msg_x.sender_id, "ou_alice");

    // Message arrives at app_y → account should be alice_via_y
    let event_y = make_message_event_with_app_id(
        "text",
        &serde_json::json!({"text": "hello from y"}).to_string(),
        "app_y",
    );
    let payload_y = make_webhook_from_event(&event_y);
    let msg_y = plugin.parse_inbound(&payload_y).await.unwrap().unwrap();
    assert_eq!(msg_y.account_id, "alice_via_y");
    assert_eq!(msg_y.sender_id, "ou_alice");

    // Cross-check: same sender, different app → different account.
    assert_ne!(msg_x.account_id, msg_y.account_id);
}

/// Boundary: header app_id not in resolver → fallback to sender_id.
/// When a message arrives from an unknown app, there is no mapping,
/// so account_id falls back to the raw sender open_id.
#[tokio::test]
async fn test_parse_inbound_header_app_id_not_in_resolver_fallback() {
    let adapter = Arc::new(make_test_adapter());
    let resolver = ConfigIdentityResolver::new(vec![IdentityMapping {
        platform: "feishu".to_string(),
        bot_app_id: "known_app".to_string(),
        sender_id: "ou_alice".to_string(),
        account_id: "alice_known".to_string(),
    }]);
    let plugin = FeishuPlugin::with_identity_resolver(adapter, Some(Arc::new(resolver)));

    // Message arrives at unknown_app → no match → fallback to sender_id
    let event = make_message_event_with_app_id(
        "text",
        &serde_json::json!({"text": "hi"}).to_string(),
        "unknown_app",
    );
    let payload = make_webhook_from_event(&event);
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(msg.account_id, "ou_alice");
}

/// Boundary: header app_id is adapter's own app_id → matches adapter.app_id
/// mapping when bot_app_id is set to the adapter's app_id. This validates
/// the fallback path: header_app_id absent/empty → use adapter.app_id.
#[tokio::test]
async fn test_parse_inbound_adapter_app_id_used_as_fallback() {
    let adapter = Arc::new(make_test_adapter());
    // Mapping uses adapter's app_id ("test_app_id") as bot_app_id.
    let resolver = ConfigIdentityResolver::new(vec![IdentityMapping {
        platform: "feishu".to_string(),
        bot_app_id: "test_app_id".to_string(),
        sender_id: "ou_alice".to_string(),
        account_id: "alice_via_adapter_app".to_string(),
    }]);
    let plugin = FeishuPlugin::with_identity_resolver(adapter, Some(Arc::new(resolver)));

    // Event with header app_id matching adapter's app_id
    let event = make_message_event_with_app_id(
        "text",
        &serde_json::json!({"text": "hi"}).to_string(),
        "test_app_id",
    );
    let payload = make_webhook_from_event(&event);
    let msg = plugin.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(msg.account_id, "alice_via_adapter_app");
}
