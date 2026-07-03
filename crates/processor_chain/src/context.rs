//! Message context, processed message, and raw message types.

use serde::{Deserialize, Serialize};

use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_llm::types::ContentBlock;

/// Result type alias for processor chain operations.
pub type Result<T> = std::result::Result<T, super::error::ProcessError>;

/// Metadata for logging a raw message snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMessageLog {
    /// Snapshot of the normalized message at this log entry.
    pub raw: NormalizedMessage,
    /// Timestamp when this snapshot was taken (Unix millis).
    pub logged_at: i64,
    /// Processor that produced this snapshot (if any).
    pub processor_name: Option<String>,
}

/// The message context carried through the processor chain.
///
/// Passed to each processor as input; processors may mutate
/// fields they are responsible for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContext {
    /// The current (possibly modified) message content.
    pub content: String,
    /// Per-processor result log, newest last.
    pub raw_message_log: Vec<RawMessageLog>,
    /// Arbitrary key-value metadata injected by processors.
    pub metadata: std::collections::HashMap<String, String>,
    /// Whether the message has been flagged to skip further processing.
    pub skip: bool,
    /// Structured content blocks (e.g., Text, Thinking, ToolUse, ToolResult).
    /// Populated on the outbound path by LLM responses; empty on inbound.
    #[serde(default)]
    pub content_blocks: Vec<ContentBlock>,
}

impl MessageContext {
    /// Creates a new context from a normalized message.
    pub fn from_normalized(msg: NormalizedMessage) -> Self {
        let logged_at = chrono::Utc::now().timestamp_millis();
        let raw_log = RawMessageLog {
            raw: msg.clone(),
            logged_at,
            processor_name: None,
        };
        Self {
            content: msg.content,
            raw_message_log: vec![raw_log],
            metadata: std::collections::HashMap::new(),
            skip: false,
            content_blocks: vec![],
        }
    }

    /// Returns a reference to the initial normalized message.
    pub fn initial_normalized(&self) -> Option<&NormalizedMessage> {
        self.raw_message_log.first().map(|l| &l.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_context_from_normalized() {
        let msg = NormalizedMessage {
            platform: "feishu".to_string(),
            sender_id: "user_1".to_string(),
            peer_id: "chat_1".to_string(),
            content: "hello".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
            thread_id: None,
            account_id: String::new(),
        };
        let ctx = MessageContext::from_normalized(msg.clone());
        assert_eq!(ctx.content, "hello");
        assert!(!ctx.skip);
        assert_eq!(ctx.metadata.len(), 0);
        assert_eq!(ctx.raw_message_log.len(), 1);
        let initial = ctx.initial_normalized().unwrap();
        assert_eq!(initial.platform, msg.platform);
        assert_eq!(initial.sender_id, msg.sender_id);
        assert_eq!(initial.peer_id, msg.peer_id);
        assert_eq!(initial.content, msg.content);
        assert_eq!(initial.timestamp, msg.timestamp);
        assert_eq!(initial.account_id, msg.account_id);
    }
}
