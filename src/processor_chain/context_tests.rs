//! Unit tests for context module — content_blocks fields.

use chrono::Utc;

use crate::processor_chain::context::{MessageContext, ProcessedMessage, RawMessage};

#[test]
fn test_message_context_content_blocks_default_empty() {
    let raw = RawMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        message_id: "msg_1".to_string(),
    };
    let ctx = MessageContext::from_raw(raw);
    assert!(ctx.content_blocks.is_empty());
}

#[test]
fn test_processed_message_content_blocks_default_empty() {
    let raw = RawMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        message_id: "msg_1".to_string(),
    };
    let processed = ProcessedMessage::from_raw(raw);
    assert!(processed.content_blocks.is_empty());
}
