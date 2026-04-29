//! Message context, processed message, and raw message types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Result type alias for processor chain operations.
pub type Result<T> = std::result::Result<T, super::error::ProcessError>;

/// A raw incoming message before any processing.
///
/// This is the input to the inbound processor chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMessage {
    /// Sender platform (e.g., "feishu", "wecom").
    pub platform: String,
    /// Sender user ID on the platform.
    pub sender_id: String,
    /// Raw message content.
    pub content: String,
    /// Timestamp when the message was received.
    pub timestamp: DateTime<Utc>,
    /// Message ID assigned by the platform.
    pub message_id: String,
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
    pub metadata: serde_json::Map<String, serde_json::Value>,
    /// Whether the message has been flagged to skip further processing.
    pub skip: bool,
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
            metadata: serde_json::Map::new(),
            skip: false,
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
    /// Final message content after all processors in the chain ran.
    pub content: String,
    /// Metadata accumulated across all processors.
    pub metadata: serde_json::Map<String, serde_json::Value>,
    /// Whether the message should be suppressed entirely.
    pub suppress: bool,
}

impl ProcessedMessage {
    /// Creates a default processed message from a raw message.
    ///
    /// This is used when the processor chain is empty (bypass).
    pub fn from_raw(raw: RawMessage) -> Self {
        Self {
            content: raw.content,
            metadata: serde_json::Map::new(),
            suppress: false,
        }
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
            content: "hello".to_string(),
            timestamp: Utc::now(),
            message_id: "msg_1".to_string(),
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
            content: "hello".to_string(),
            timestamp: Utc::now(),
            message_id: "msg_1".to_string(),
        };
        let processed = ProcessedMessage::from_raw(raw);
        assert_eq!(processed.content, "hello");
        assert!(!processed.suppress);
        assert_eq!(processed.metadata.len(), 0);
    }
}
