//! Shared data types for the gateway crate.

use closeclaw_common::im_plugin::{MediaRef, MessageType};
use closeclaw_llm::types::ContentBlock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Type alias for the output channel sender used across session handler modules.
pub(crate) type OutputTx = Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>;

/// DM session scope - controls how session keys are partitioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DmScope {
    /// Single shared session for all peers on a channel (backward compatible)
    Main,
    /// One session per peer pair (from → to)
    PerPeer,
    /// One session per channel + peer pair
    PerChannelPeer,
    /// One session per account + channel + peer pair
    PerAccountChannelPeer,
    /// One session per channel + sender (excludes `to` field).
    ///
    /// Used when agent-level isolation is provided by a per-agent
    /// [`SessionManager`] — the session key is `{channel}:{from}`, so
    /// different agents sharing the same channel naturally stay isolated
    /// without embedding `agent_id` in the key.
    PerChannelSender,
}

#[allow(clippy::derivable_impls)]
impl Default for DmScope {
    fn default() -> Self {
        DmScope::PerAccountChannelPeer
    }
}

impl DmScope {
    /// Compute a session key for the given context.
    ///
    /// Format: `{timestamp_ms}-{sha256_hex(routing_fields)}`
    /// where `routing_fields` varies by scope variant.
    pub fn compute_session_key(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
        timestamp_ms: i64,
    ) -> String {
        let routing_fields = match self {
            DmScope::Main => {
                format!("{}:{}:{}", channel, message.to, timestamp_ms)
            }
            DmScope::PerPeer => {
                format!("{}:{}:{}", message.from, message.to, timestamp_ms)
            }
            DmScope::PerChannelPeer => {
                format!(
                    "{}:{}:{}:{}",
                    channel, message.from, message.to, timestamp_ms
                )
            }
            DmScope::PerAccountChannelPeer => {
                let acc = account_id.unwrap_or("default");
                format!(
                    "{}:{}:{}:{}:{}",
                    channel, message.from, message.to, acc, timestamp_ms
                )
            }
            DmScope::PerChannelSender => {
                format!("{}:{}:{}", channel, message.from, timestamp_ms)
            }
        };
        let hash = Sha256::digest(routing_fields.as_bytes());
        format!("{}-{:x}", timestamp_ms, hash)
    }
}

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
}

/// Gateway configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub name: String,
    #[serde(default)]
    pub rate_limit_per_minute: u32,
    #[serde(default)]
    pub max_message_size: usize,
    #[serde(default)]
    pub dm_scope: DmScope,
    /// Directory for raw inbound log files.
    /// When `None` (default), raw logging is disabled.
    #[serde(default)]
    pub raw_log_dir: Option<std::path::PathBuf>,
    /// Maximum number of messages the inbound queue can buffer.
    /// Defaults to 64.
    #[serde(default = "default_inbound_queue_capacity")]
    pub inbound_queue_capacity: usize,
}

fn default_inbound_queue_capacity() -> usize {
    64
}

#[allow(clippy::derivable_impls)]
impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            rate_limit_per_minute: 0,
            max_message_size: 0,
            dm_scope: DmScope::default(),
            raw_log_dir: None,
            inbound_queue_capacity: default_inbound_queue_capacity(),
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
