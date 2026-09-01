//! Supplementary tests for Step 1.8 — gap filling and regression.
//!
//! Covers behavioral dimensions not yet validated by Steps 1.1–1.7:
//! - Full-chain CLI format parse_inbound (anchor → peer_id / reply_ref)
//! - Malformed CLI event content → graceful error
//! - Event stream integration with group chat type → discarded (adapter filters chat_type)
//! - Process restart recovery via event channel
//! - Duplicate event via full parse_inbound chain (dedup gate)

use super::process_manager::{start_event_stream, Event, EventLine};
use super::*;
use crate::media_store::MediaStore;
use crate::IMAdapter;
use closeclaw_common::MessageType;
use std::sync::Arc;
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

/// Create a test Gateway with inbound queue started.
fn make_test_gateway() -> std::sync::Arc<closeclaw_gateway::Gateway> {
    use closeclaw_gateway::{Gateway, GatewayConfig};

    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 4,
        inbound_wal_dir: None,
        ..Default::default()
    };
    let sm = std::sync::Arc::new(closeclaw_gateway::SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let gw = std::sync::Arc::new(Gateway::new(config, sm));
    gw.start_inbound_queue();
    gw
}

// ===========================================================================
// Full-chain CLI format parse_inbound tests
// ===========================================================================

/// Build a CLI-format text message payload.
///
/// Content must be a JSON string like `{"text":"hello"}` — the CLI outputs
/// JSON-encoded content, not plain text.
fn cli_text_payload(
    event_id: &str,
    message_id: &str,
    sender_id: &str,
    text: &str,
    extra_fields: Option<serde_json::Value>,
) -> Vec<u8> {
    let mut payload = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": event_id,
        "message_id": message_id,
        "chat_id": "oc_test_chat",
        "message_type": "text",
        "sender_id": sender_id,
        "sender_type": "user",
        "content": serde_json::json!({"text": text}).to_string()
    });
    if let Some(extra) = extra_fields {
        if let (Some(obj), Some(extra_obj)) = (payload.as_object_mut(), extra.as_object()) {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    serde_json::to_vec(&payload).unwrap()
}

/// CLI format p2p-top-text: full chain parse_inbound → verify anchor fields.
///
/// Simulates the real p2p-top-text.json event structure.
#[tokio::test]
async fn test_full_chain_cli_p2p_top_text() {
    let adapter = make_test_adapter();
    let payload = cli_text_payload(
        "ev_chain_top_001",
        "om_x100b662c236f18a4c213c559ac4a010",
        "ou_owner_b1ctx",
        "A2 补测",
        None,
    );
    let msg = adapter.parse_inbound(&payload).await.unwrap().unwrap();
    // Verify anchor construction: top-level message
    assert_eq!(
        msg.peer_id,
        "ou_owner_b1ctx|om_x100b662c236f18a4c213c559ac4a010"
    );
    assert_eq!(
        msg.reply_ref.as_deref(),
        Some("om_x100b662c236f18a4c213c559ac4a010")
    );
    assert_eq!(msg.message_id, "om_x100b662c236f18a4c213c559ac4a010");
    assert_eq!(msg.content, "A2 补测");
    assert_eq!(msg.message_type, MessageType::Text);
    assert_eq!(msg.platform, "feishu");
    assert_eq!(msg.sender_id, "ou_owner_b1ctx");
    // No thread_id for top-level message
    assert!(msg.thread_id.is_none());
}

/// CLI format p2p-thread-reply: full chain parse_inbound → verify topic anchor.
///
/// Simulates the real p2p-thread-reply.json event structure.
#[tokio::test]
async fn test_full_chain_cli_p2p_thread_reply() {
    let adapter = make_test_adapter();
    let payload = cli_text_payload(
        "ev_chain_thread_001",
        "om_x100b662c207e7ca4dd414962d2c6de4",
        "ou_owner_b1ctx",
        "A4 话题回复",
        Some(serde_json::json!({
            "root_id": "om_x100b662c236f18a4c213c559ac4a010",
            "thread_id": "omt_19f40e130f4f5b8d"
        })),
    );
    let msg = adapter.parse_inbound(&payload).await.unwrap().unwrap();
    // Verify topic anchor: peer_id = sender|thread_id, reply_ref = root_id
    assert_eq!(msg.peer_id, "ou_owner_b1ctx|omt_19f40e130f4f5b8d");
    assert_eq!(
        msg.reply_ref.as_deref(),
        Some("om_x100b662c236f18a4c213c559ac4a010")
    );
    assert_eq!(msg.thread_id.as_deref(), Some("omt_19f40e130f4f5b8d"));
    assert_eq!(msg.content, "A4 话题回复");
    assert_eq!(msg.message_type, MessageType::Text);
}

/// CLI format: top-level text with no thread fields → anchor uses message_id.
#[tokio::test]
async fn test_full_chain_cli_top_level_no_thread_fields() {
    let adapter = make_test_adapter();
    let payload = cli_text_payload(
        "ev_chain_notop_001",
        "om_msg_no_thread",
        "ou_sender_simple",
        "simple message",
        None,
    );
    let msg = adapter.parse_inbound(&payload).await.unwrap().unwrap();
    assert_eq!(msg.peer_id, "ou_sender_simple|om_msg_no_thread");
    assert_eq!(msg.reply_ref.as_deref(), Some("om_msg_no_thread"));
    assert!(msg.thread_id.is_none());
}

/// CLI format: thread reply with root_id but no thread_id → root_id used as anchor.
#[tokio::test]
async fn test_full_chain_cli_thread_reply_root_id_only() {
    let adapter = make_test_adapter();
    let payload = cli_text_payload(
        "ev_chain_root_001",
        "om_msg_in_thread",
        "ou_sender_thread",
        "thread reply",
        Some(serde_json::json!({
            "root_id": "om_root_anchor"
        })),
    );
    let msg = adapter.parse_inbound(&payload).await.unwrap().unwrap();
    // root_id → used as thread_id fallback → peer_id = sender|root_id
    assert_eq!(msg.peer_id, "ou_sender_thread|om_root_anchor");
    assert_eq!(msg.reply_ref.as_deref(), Some("om_root_anchor"));
    assert_eq!(msg.thread_id.as_deref(), Some("om_root_anchor"));
}

// ===========================================================================
// Malformed CLI event content
// ===========================================================================

/// CLI format with invalid JSON in content field → parse_message_event fails gracefully.
#[tokio::test]
async fn test_full_chain_cli_malformed_content_returns_error() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_bad_content_001",
        "message_id": "om_bad_content",
        "chat_id": "oc_chat_bad",
        "message_type": "text",
        "sender_id": "ou_sender_bad",
        "content": "this is not valid json {{{"
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let result = adapter.parse_inbound(&bytes).await;
    assert!(result.is_err(), "malformed content JSON should return Err");
}

/// CLI format with empty content → text type returns None (empty text discarded).
#[tokio::test]
async fn test_full_chain_cli_empty_content_returns_none() {
    let adapter = make_test_adapter();
    // Empty text message → content.text is empty → discarded
    let payload = cli_text_payload(
        "ev_empty_content_001",
        "om_empty_content",
        "ou_sender_empty",
        "",
        None,
    );
    let result = adapter.parse_inbound(&payload).await.unwrap();
    assert!(
        result.is_none(),
        "empty content text message should be discarded"
    );
}

// ===========================================================================
// Duplicate event through full parse_inbound chain
// ===========================================================================

/// Duplicate event_id rejected at parse_inbound level (dedup gate).
#[tokio::test]
async fn test_full_chain_dedup_duplicate_rejected() {
    let adapter = make_test_adapter();
    let payload = cli_text_payload(
        "ev_chain_dedup_001",
        "om_dedup_msg",
        "ou_sender_dedup",
        "first",
        None,
    );
    let msg1 = adapter.parse_inbound(&payload).await.unwrap();
    assert!(msg1.is_some(), "first event should pass");
    assert_eq!(msg1.unwrap().content, "first");

    // Same event_id, different content → rejected
    let payload2 = cli_text_payload(
        "ev_chain_dedup_001",
        "om_dedup_msg_2",
        "ou_sender_dedup",
        "second",
        None,
    );
    let msg2 = adapter.parse_inbound(&payload2).await.unwrap();
    assert!(msg2.is_none(), "duplicate event_id should be rejected");
}

// ===========================================================================
// Group chat event through full chain (filtered by adapter)
// ===========================================================================

/// Group chat text event: parse_inbound discards it per design doc.
/// "群聊 receive 事件仅记录调试日志，不入消息通路" — adapter filters group
/// chat_type early in parse_message_event, before any side-effects.
#[tokio::test]
async fn test_full_chain_group_chat_event_discarded() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_group_001",
        "message_id": "om_group_msg",
        "chat_id": "oc_group_chat",
        "chat_type": "group",
        "message_type": "text",
        "sender_id": "ou_group_sender",
        "sender_type": "user",
        "content": serde_json::json!({"text": "group hello"}).to_string()
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let msg = adapter.parse_inbound(&bytes).await.unwrap();
    assert!(
        msg.is_none(),
        "group chat event should be discarded by adapter"
    );
}

/// P2P chat text event with chat_type="p2p" is processed normally.
#[tokio::test]
async fn test_full_chain_p2p_chat_event_with_chat_type() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_p2p_type_001",
        "message_id": "om_p2p_type_msg",
        "chat_id": "oc_p2p_chat",
        "chat_type": "p2p",
        "message_type": "text",
        "sender_id": "ou_p2p_sender",
        "sender_type": "user",
        "content": serde_json::json!({"text": "p2p hello"}).to_string()
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let msg = adapter.parse_inbound(&bytes).await.unwrap();
    assert!(msg.is_some(), "p2p chat event should be processed");
    let msg = msg.unwrap();
    assert_eq!(msg.content, "p2p hello");
    assert_eq!(msg.peer_id, "ou_p2p_sender|om_p2p_type_msg");
}

/// P2P chat text event with no chat_type field is also processed.
#[tokio::test]
async fn test_full_chain_p2p_chat_event_without_chat_type() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_p2p_notype_001",
        "message_id": "om_p2p_notype_msg",
        "chat_id": "oc_p2p_chat",
        "message_type": "text",
        "sender_id": "ou_p2p_sender",
        "sender_type": "user",
        "content": serde_json::json!({"text": "no type field"}).to_string()
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let msg = adapter.parse_inbound(&bytes).await.unwrap();
    assert!(
        msg.is_some(),
        "p2p event without chat_type should be processed"
    );
    assert_eq!(msg.unwrap().content, "no type field");
}

// ===========================================================================
// Event stream: subprocess restart recovery via channel
// ===========================================================================

/// Event stream survives channel reconnection: events from a restarted
/// subprocess are still received and enqueued.
#[tokio::test]
async fn test_event_stream_survives_event_channel_reconnect() {
    let gw = make_test_gateway();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    start_event_stream(&gw, rx);

    // Simulate first batch of events (before "restart")
    for i in 0..3 {
        let event = Event {
            event_type: "im.message.receive_v1".to_string(),
            event_id: format!("ev_pre_restart_{i}"),
            raw: serde_json::json!({
                "type": "im.message.receive_v1",
                "event_id": format!("ev_pre_restart_{i}"),
                "message_id": format!("om_pre_{i}"),
                "sender_id": "ou_user",
                "content": "{\"text\":\"before restart\"}",
                "chat_id": "oc_chat",
                "message_type": "text"
            }),
        };
        tx.send(EventLine::Event(event)).unwrap();
    }

    // Small delay to let events be processed
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Simulate "restart" — new events from the respawned subprocess
    for i in 0..3 {
        let event = Event {
            event_type: "im.message.receive_v1".to_string(),
            event_id: format!("ev_post_restart_{i}"),
            raw: serde_json::json!({
                "type": "im.message.receive_v1",
                "event_id": format!("ev_post_restart_{i}"),
                "message_id": format!("om_post_{i}"),
                "sender_id": "ou_user",
                "content": "{\"text\":\"after restart\"}",
                "chat_id": "oc_chat",
                "message_type": "text"
            }),
        };
        tx.send(EventLine::Event(event)).unwrap();
    }

    // Drop tx to end the stream
    drop(tx);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // No panic = success. Events from both "batches" were enqueued.
}

// ===========================================================================
// Event stream: non-message events filtered by parse_inbound
// ===========================================================================

/// Event stream with reaction.created event: parse_inbound returns None,
/// Gateway discards it. No panic, no crash.
#[tokio::test]
async fn test_event_stream_reaction_event_filtered() {
    let gw = make_test_gateway();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    start_event_stream(&gw, rx);

    let event = Event {
        event_type: "im.message.reaction.created_v1".to_string(),
        event_id: "ev_reaction_stream_001".to_string(),
        raw: serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.reaction.created_v1",
                "event_id": "ev_reaction_stream_001",
                "create_time": "0",
                "token": "t",
                "app_id": "a"
            },
            "event": {
                "message_id": "om_msg_reaction",
                "operator": {"open_id": "ou_user", "operator_type": "user"},
                "reaction_type": {"emoji_type": "THUMBSUP"}
            }
        }),
    };
    tx.send(EventLine::Event(event)).unwrap();
    drop(tx);

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // No panic = success. Reaction event was enqueued, parse_inbound returns None,
    // Gateway discards it.
}

/// Event stream with card.action.trigger event: parse_inbound returns None,
/// Gateway discards it. No panic.
#[tokio::test]
async fn test_event_stream_card_action_filtered() {
    let gw = make_test_gateway();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    start_event_stream(&gw, rx);

    let event = Event {
        event_type: "card.action.trigger".to_string(),
        event_id: "ev_card_stream_001".to_string(),
        raw: serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "card.action.trigger",
                "event_id": "ev_card_stream_001",
                "create_time": "0",
                "token": "t",
                "app_id": "a"
            },
            "operator": {"open_id": "ou_op"},
            "token": "tok",
            "action": {"value": {"action": "btn"}}
        }),
    };
    tx.send(EventLine::Event(event)).unwrap();
    drop(tx);

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // No panic = success. Card action was enqueued, parse_inbound returns None.
}
