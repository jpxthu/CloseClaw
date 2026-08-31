//! Message context, processed message, and raw message types.

use std::collections::HashMap;

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
    pub metadata: HashMap<String, String>,
    /// Whether the message has been flagged to skip further processing.
    pub skip: bool,
    /// Structured content blocks (e.g., Text, Thinking, ToolUse, ToolResult).
    /// Populated on the outbound path by LLM responses; empty on inbound.
    #[serde(default)]
    pub content_blocks: Vec<ContentBlock>,
}

impl MessageContext {
    /// Creates a new context from a normalized message.
    ///
    /// Copies `message_type` and `unavailable_media` from the normalized
    /// message into the initial metadata, as specified by the design doc:
    /// "message_type 与 unavailable_media 由链调度环节在进链时从
    /// NormalizedMessage 复制到 ProcessedMessage.metadata".
    pub fn from_normalized(msg: NormalizedMessage) -> Self {
        let logged_at = chrono::Utc::now().timestamp_millis();
        let raw_log = RawMessageLog {
            raw: msg.clone(),
            logged_at,
            processor_name: None,
        };
        let mut metadata = HashMap::new();
        // Design doc: chain dispatcher copies these two keys into metadata
        // before the chain runs.
        metadata.insert(
            "message_type".to_string(),
            serde_json::to_string(&msg.message_type).unwrap_or_default(),
        );
        metadata.insert(
            "unavailable_media".to_string(),
            serde_json::to_string(&msg.unavailable_media).unwrap_or_default(),
        );
        Self {
            content: msg.content,
            raw_message_log: vec![raw_log],
            metadata,
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
    use closeclaw_common::im_plugin::MessageType;

    #[test]
    fn test_message_context_from_normalized() {
        let msg = NormalizedMessage {
            platform: "feishu".to_string(),
            sender_id: "user_1".to_string(),
            peer_id: "chat_1".to_string(),
            content: "hello".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            message_type: MessageType::Text,
            media_refs: Vec::new(),
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
        };
        let ctx = MessageContext::from_normalized(msg.clone());
        assert_eq!(ctx.content, "hello");
        assert!(!ctx.skip);
        // Design doc: chain dispatcher copies message_type into metadata
        assert_eq!(
            ctx.metadata.get("message_type").map(|s| s.as_str()),
            Some("\"text\""),
            "message_type should be copied into metadata by from_normalized"
        );
        // unavailable_media defaults to empty array
        assert_eq!(
            ctx.metadata.get("unavailable_media").map(|s| s.as_str()),
            Some("[]"),
            "unavailable_media should be copied into metadata"
        );
        assert_eq!(ctx.raw_message_log.len(), 1);
        let initial = ctx.initial_normalized().unwrap();
        assert_eq!(initial.platform, msg.platform);
        assert_eq!(initial.sender_id, msg.sender_id);
        assert_eq!(initial.peer_id, msg.peer_id);
        assert_eq!(initial.content, msg.content);
        assert_eq!(initial.timestamp, msg.timestamp);
        assert_eq!(initial.account_id, msg.account_id);
    }

    #[test]
    fn test_from_normalized_unavailable_media_copied() {
        let msg = NormalizedMessage {
            platform: "feishu".to_string(),
            sender_id: "u1".to_string(),
            peer_id: "c1".to_string(),
            content: "check this".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            message_type: Default::default(),
            media_refs: Vec::new(),
            thread_id: None,
            account_id: String::new(),
            unavailable_media: vec!["file_a".to_string(), "file_b".to_string()],
            ..Default::default()
        };
        let ctx = MessageContext::from_normalized(msg);
        let um = ctx.metadata.get("unavailable_media").unwrap();
        assert_eq!(um, "[\"file_a\",\"file_b\"]");
    }
}
