//! Shared data types for the gateway crate.

use closeclaw_common::im_plugin::{MediaRef, MessageType};
use closeclaw_llm::types::ContentBlock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Type alias for the output channel sender used across session handler modules.
pub(crate) type OutputTx = Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>;

/// Compute a session key for the given context.
///
/// Re-exported from [`closeclaw_common::session_key`].
pub use closeclaw_common::session_key::compute_session_key;

/// Internal message representation - all IM messages are converted to this
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub content: String,
    pub channel: String,
    pub timestamp: i64,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    /// 出站消息的平台标识（如 "feishu"、"telegram"）
    #[serde(default)]
    pub platform: Option<String>,
    /// DSL 解析结果（序列化 JSON 字符串）
    #[serde(default)]
    pub dsl_result: Option<String>,
    /// 出站消息的内容块（序列化 JSON 字符串，ContentBlock[]）
    #[serde(default)]
    pub content_blocks: Option<String>,
}

/// Gateway configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub name: String,
    #[serde(default)]
    pub rate_limit_per_minute: u32,
    #[serde(default)]
    pub max_message_size: usize,
    /// Directory for raw inbound log files.
    /// When `None` (default), raw logging is disabled.
    #[serde(default)]
    pub raw_log_dir: Option<std::path::PathBuf>,
    /// Maximum number of messages the inbound queue can buffer.
    /// Defaults to 256.
    #[serde(default = "default_inbound_queue_capacity")]
    pub inbound_queue_capacity: usize,
    /// Directory for inbound WAL persistence.
    /// When `None` (default), WAL persistence is disabled and the queue
    /// behaves as an in-memory buffer only.
    #[serde(default)]
    pub inbound_wal_dir: Option<std::path::PathBuf>,
}

pub(crate) fn default_inbound_queue_capacity() -> usize {
    256
}

#[allow(clippy::derivable_impls)]
impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            rate_limit_per_minute: 0,
            max_message_size: 0,
            raw_log_dir: None,
            inbound_queue_capacity: default_inbound_queue_capacity(),
            inbound_wal_dir: None,
        }
    }
}

/// Session - represents an active conversation
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel: String,
    pub created_at: i64,
    /// Nesting depth. 0 for root sessions, parent.depth + 1 for child sessions.
    pub depth: u32,
}

/// Groups inbound message fields into a single struct.
#[derive(Debug, Clone)]
pub struct InboundChainInput {
    pub platform: String,
    pub sender_id: String,
    pub peer_id: String,
    pub content: String,
    pub message_id: String,
    pub timestamp_ms: i64,
    pub account_id: Option<String>,
    /// Thread/topic ID for threaded replies (optional).
    pub thread_id: Option<String>,
    /// Message type (text, image, file, audio).
    pub message_type: MessageType,
    /// Media attachment references.
    pub media_refs: Vec<MediaRef>,
    /// Chat/group name (e.g. Feishu group title), or None.
    pub chat_name: Option<String>,
    /// Trace ID for distributed tracing, or None.
    pub trace_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("Unknown channel: {0}")]
    UnknownChannel(String),
    #[error("Message too large")]
    MessageTooLarge,
    #[error("Adapter error: {0}")]
    AdapterError(String),
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Missing session ID in message metadata")]
    MissingSessionId,
    #[error("No routing key: both session_key and session_id missing from metadata")]
    NoRoutingKey,

    #[error("Outbound error: {0}")]
    OutboundError(String),

    /// Streaming error that preserves partially received content blocks.
    ///
    /// When a [`StreamEvent::Error`](closeclaw_llm::types::StreamEvent::Error)
    /// arrives mid-stream, any `ContentBlock`s accumulated so far are carried
    /// here rather than silently discarded, allowing callers to log or inspect
    /// the partial output.
    #[error("Streaming error: {message}")]
    StreamError {
        message: String,
        /// Content blocks received before the error occurred.
        partial_content: Vec<ContentBlock>,
    },
}

impl From<closeclaw_common::AdapterError> for GatewayError {
    fn from(e: closeclaw_common::AdapterError) -> Self {
        GatewayError::AdapterError(e.to_string())
    }
}
