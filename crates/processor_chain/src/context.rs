//! Message context, processed message, and raw message types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use closeclaw_llm::types::ContentBlock;

/// Result type alias for processor chain operations.
pub type Result<T> = std::result::Result<T, super::error::ProcessError>;

/// A raw incoming message before any processing.
///
/// This is the input to the inbound processor chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMessage {
    /// Sender platform (e.g., "feishu", "wecom", "terminal").
    pub platform: String,
    /// Sender user ID on the platform.
    pub sender_id: String,
    /// Peer / endpoint identifier (e.g., chat_id for DMs, "cli" for terminal).
    ///
    /// Participates in session key computation via [`SessionRouter`].
    #[serde(default)]
    pub peer_id: String,
    /// Raw message content.
    pub content: String,
    /// Timestamp when the message was received.
    pub timestamp: DateTime<Utc>,
    /// Message ID assigned by the platform.
    pub message_id: String,
    /// Optional account ID filled by IM Adapter via identity mapping.
    #[serde(default)]
    pub account_id: Option<String>,
}

/// Metadata for logging a raw message snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMessageLog {
    /// Snapshot of the raw message at this log entry.
    pub raw: RawMessage,
    /// Timestamp when this snapshot was taken.
    pub logged_at: DateTime<Utc>,
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
    /// Creates a new context from a raw message.
    pub fn from_raw(raw: RawMessage) -> Self {
        let logged_at = Utc::now();
        let raw_log = RawMessageLog {
            raw: raw.clone(),
            logged_at,
            processor_name: None,
        };
        Self {
            content: raw.content,
            raw_message_log: vec![raw_log],
            metadata: std::collections::HashMap::new(),
            skip: false,
            content_blocks: vec![],
        }
    }

    /// Returns a reference to the initial raw message.
    pub fn initial_raw(&self) -> Option<&RawMessage> {
        self.raw_message_log.first().map(|l| &l.raw)
    }
}

/// The result of running the full processor chain.
///
/// Produced after either the inbound or outbound chain completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedMessage {
    /// Structured content blocks carried from the chain.
    #[serde(default)]
    pub content_blocks: Vec<ContentBlock>,
    /// Metadata accumulated across all processors.
    pub metadata: std::collections::HashMap<String, String>,
}

impl ProcessedMessage {
    /// Creates a default processed message from a raw message.
    ///
    /// This is used when the processor chain is empty (bypass).
    pub fn from_raw(raw: RawMessage) -> Self {
        Self {
            content_blocks: vec![ContentBlock::Text(raw.content)],
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Returns the first text content block's text, if present.
    pub fn text_content(&self) -> Option<&str> {
        self.content_blocks.iter().find_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_context_from_raw() {
        let raw = RawMessage {
            platform: "feishu".to_string(),
            sender_id: "user_1".to_string(),
            peer_id: "chat_1".to_string(),
            content: "hello".to_string(),
            timestamp: Utc::now(),
            message_id: "msg_1".to_string(),
            account_id: None,
        };
        let ctx = MessageContext::from_raw(raw.clone());
        assert_eq!(ctx.content, "hello");
        assert!(!ctx.skip);
        assert_eq!(ctx.metadata.len(), 0);
        assert_eq!(ctx.raw_message_log.len(), 1);
        assert_eq!(ctx.initial_raw(), Some(&raw));
    }

    #[test]
    fn test_processed_message_from_raw() {
        let raw = RawMessage {
            platform: "feishu".to_string(),
            sender_id: "user_1".to_string(),
            peer_id: "chat_1".to_string(),
            content: "hello".to_string(),
            timestamp: Utc::now(),
            message_id: "msg_1".to_string(),
            account_id: None,
        };
        let processed = ProcessedMessage::from_raw(raw);
        assert_eq!(processed.text_content(), Some("hello"));
        assert!(processed.content_blocks.len() == 1);
        assert_eq!(processed.metadata.len(), 0);
    }
}
