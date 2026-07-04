//! Unit tests for context module — content_blocks fields.

use crate::processor_chain::context::MessageContext;
use closeclaw_common::im_plugin::NormalizedMessage;

fn make_normalized() -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "chat_1".to_string(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: String::new(),
    }
}

#[test]
fn test_message_context_content_blocks_default_empty() {
    let msg = make_normalized();
    let ctx = MessageContext::from_normalized(msg);
    assert!(ctx.content_blocks.is_empty());
}

#[test]
fn test_processed_message_content_blocks_from_normalized() {
    let msg = make_normalized();
    let ctx = MessageContext::from_normalized(msg);
    let processed = closeclaw_common::processor::ProcessedMessage::from_raw_content(ctx.content);
    assert_eq!(processed.content_blocks.len(), 1);
    assert_eq!(processed.text_content(), Some("hello"));
}
