//! Unit tests for context module — content_blocks fields.

use chrono::Utc;

use crate::processor_chain::context::{MessageContext, RawMessage};

#[test]
fn test_message_context_content_blocks_default_empty() {
    let raw = RawMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "chat_1".to_string(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        message_id: "msg_1".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    assert!(ctx.content_blocks.is_empty());
}

#[test]
fn test_processed_message_content_blocks_from_raw() {
    let raw = RawMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "chat_1".to_string(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        message_id: "msg_1".to_string(),
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let processed = closeclaw_common::processor::ProcessedMessage::from_raw_content(ctx.content);
    assert_eq!(processed.content_blocks.len(), 1);
    assert_eq!(processed.text_content(), Some("hello"));
}
