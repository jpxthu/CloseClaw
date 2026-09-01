//! Shared data types for the gateway crate.

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
    /// 出站定向回复引用（IM 渠道定向投递的靶标 ID）
    #[serde(default)]
    pub reply_ref: Option<String>,
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
    /// Defaults to `~/.closeclaw/inbound_wal` (aligns with design doc:
    /// messages are persisted on enqueue by default).
    /// Setting to `None` explicitly disables WAL persistence.
    #[serde(default = "default_inbound_wal_dir")]
    pub inbound_wal_dir: Option<std::path::PathBuf>,
    /// Bot → Agent binding map.
    /// Key is the bot identifier (peer_id), value is the agent_id to route to.
    /// When a message's peer_id matches a key, the bound agent_id is used;
    /// otherwise peer_id is used as agent_id (backward compatible).
    #[serde(default)]
    pub bot_agent_bindings: HashMap<String, String>,
}

pub(crate) fn default_inbound_queue_capacity() -> usize {
    256
}

/// Default inbound WAL directory: `~/.closeclaw/inbound_wal`.
///
/// Aligns with the production daemon convention
/// (`config_dir = ~/.closeclaw`, WAL subdirectory `inbound_wal`).
pub(crate) fn default_inbound_wal_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".closeclaw").join("inbound_wal"))
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
            inbound_wal_dir: default_inbound_wal_dir(),
            bot_agent_bindings: HashMap::new(),
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
