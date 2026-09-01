//! Unit tests for `normalize_cli_event` (Step 1.1).
//!
//! Covers:
//! - CLI event fields correctly mapped to FeishuEvent
//! - Missing optional fields use sensible defaults
//! - Thread reply fields (thread_id, root_id, parent_id) preserved
//! - Group chat events (chat_id populated)
//! - Reaction and bot events preserved via raw JSON

use super::process_manager::normalize_cli_event;

// ===========================================================================
// CLI format → FeishuEvent mapping tests
// ===========================================================================

#[test]
fn test_normalize_cli_receive_message() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_001",
        "message_id": "om_msg_001",
        "sender_id": "ou_alice",
        "sender_type": "user",
        "content": "{\"text\":\"hello\"}",
        "chat_id": "oc_chat_001",
        "message_type": "text",
        "create_time": "1700000000",
        "app_id": "cli_abc"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.header.event_type, "im.message.receive_v1");
    assert_eq!(event.header.event_id, "ev_001");
    assert_eq!(event.header.create_time, "1700000000");
    assert_eq!(event.header.app_id, "cli_abc");
    assert_eq!(event.event.message_id.as_deref(), Some("om_msg_001"));
    assert_eq!(event.event.sender.sender_id.open_id, "ou_alice");
    assert_eq!(event.event.sender.sender_type, "user");
    assert_eq!(event.event.content, "{\"text\":\"hello\"}");
    assert_eq!(event.event.chat_id, "oc_chat_001");
    assert_eq!(event.event.message_type, "text");
}

#[test]
fn test_normalize_cli_thread_reply() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_002",
        "message_id": "om_msg_002",
        "sender_id": "ou_bob",
        "content": "{\"text\":\"reply\"}",
        "chat_id": "oc_chat_002",
        "message_type": "text",
        "thread_id": "om_thread_123",
        "root_id": "om_root_456",
        "parent_id": "om_parent_789"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.event.thread_id.as_deref(), Some("om_thread_123"));
    assert_eq!(event.event.root_id.as_deref(), Some("om_root_456"));
    assert_eq!(event.event.parent_id.as_deref(), Some("om_parent_789"));
}

#[test]
fn test_normalize_cli_missing_optional_fields() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_003",
        "sender_id": "ou_user",
        "content": "{\"text\":\"hi\"}",
        "chat_id": "oc_chat_003",
        "message_type": "text"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert!(event.event.message_id.is_none());
    assert!(event.event.thread_id.is_none());
    assert!(event.event.root_id.is_none());
    assert!(event.event.parent_id.is_none());
    assert_eq!(event.header.create_time, "");
    assert_eq!(event.header.app_id, "");
    assert_eq!(event.event.sender.sender_type, "user");
}

#[test]
fn test_normalize_cli_sender_type_defaults_to_user() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_004",
        "sender_id": "ou_user",
        "content": "{}",
        "chat_id": "oc_chat_004",
        "message_type": "text"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.event.sender.sender_type, "user");
}

#[test]
fn test_normalize_cli_bot_sender_type() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_005",
        "sender_id": "ou_bot",
        "sender_type": "bot",
        "content": "{}",
        "chat_id": "oc_chat_005",
        "message_type": "text"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.event.sender.sender_type, "bot");
}

#[test]
fn test_normalize_cli_group_chat_event() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_006",
        "message_id": "om_msg_006",
        "sender_id": "ou_user",
        "content": "{\"text\":\"group message\"}",
        "chat_id": "oc_group_chat",
        "message_type": "text"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.event.chat_id, "oc_group_chat");
}

#[test]
fn test_normalize_cli_post_message_type() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_007",
        "message_id": "om_msg_007",
        "sender_id": "ou_user",
        "content": "{\"title\":\"T\",\"content\":[[{\"tag\":\"text\",\"text\":\"body\"}]]}",
        "chat_id": "oc_chat_007",
        "message_type": "post"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.event.message_type, "post");
    assert!(event.event.content.contains("body"));
}

#[test]
fn test_normalize_cli_file_message_type() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_008",
        "message_id": "om_msg_008",
        "sender_id": "ou_user",
        "content": "{\"file_key\":\"fk_123\",\"file_name\":\"doc.pdf\"}",
        "chat_id": "oc_chat_008",
        "message_type": "file"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.event.message_type, "file");
}

// ===========================================================================
// CLI format detection and return value
// ===========================================================================

#[test]
fn test_normalize_cli_returns_none_for_non_cli_format() {
    // Webhook format (no top-level "type") should return None
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_type": "im.message.receive_v1",
            "event_id": "ev_009"
        },
        "event": {}
    });
    let result = normalize_cli_event(&raw);
    assert!(result.is_none(), "webhook format should return None");
}

#[test]
fn test_normalize_cli_returns_none_for_missing_type() {
    let raw = serde_json::json!({
        "event_id": "ev_010",
        "sender_id": "ou_user"
    });
    let result = normalize_cli_event(&raw);
    assert!(result.is_none(), "missing 'type' should return None");
}

#[test]
fn test_normalize_cli_returns_none_for_empty_json() {
    let raw = serde_json::json!({});
    let result = normalize_cli_event(&raw);
    assert!(result.is_none(), "empty JSON should return None");
}

// ===========================================================================
// Reaction and bot events (non-message types)
// ===========================================================================

#[test]
fn test_normalize_cli_reaction_event_preserved() {
    let raw = serde_json::json!({
        "type": "im.message.reaction.created_v1",
        "event_id": "ev_reaction",
        "message_id": "om_msg_reaction",
        "sender_id": "ou_user",
        "content": "{}",
        "chat_id": "oc_chat_reaction",
        "message_type": ""
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.header.event_type, "im.message.reaction.created_v1");
}

#[test]
fn test_normalize_cli_card_action_event_preserved() {
    let raw = serde_json::json!({
        "type": "card.action.trigger",
        "event_id": "ev_card",
        "sender_id": "ou_operator",
        "content": "{}",
        "chat_id": "oc_chat_card",
        "message_type": ""
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert_eq!(event.header.event_type, "card.action.trigger");
}

// ===========================================================================
// Schema field
// ===========================================================================

#[test]
fn test_normalize_cli_schema_is_empty() {
    let raw = serde_json::json!({
        "type": "im.message.receive_v1",
        "event_id": "ev_011",
        "sender_id": "ou_user",
        "content": "{}",
        "chat_id": "oc_chat",
        "message_type": "text"
    });
    let event = normalize_cli_event(&raw).expect("should parse");
    assert!(
        event.schema.is_empty(),
        "CLI events should have empty schema"
    );
}
