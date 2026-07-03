//! Gateway shared data types.
//!
//! Core message, configuration, and error types used across the gateway
//! and dependent crates. These types were extracted from `src/gateway/`
//! to enable clean workspace crate boundaries.

use crate::im_plugin::{MediaRef, MessageType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// Internal message representation — all IM messages are converted to this.
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

// ---------------------------------------------------------------------------
// DmScope
// ---------------------------------------------------------------------------

/// DM session scope — controls how session keys are partitioned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DmScope {
    /// Single shared session for all peers on a channel (backward compatible).
    Main,
    /// One session per peer pair (from → to).
    PerPeer,
    /// One session per channel + peer pair.
    PerChannelPeer,
    /// One session per account + channel + peer pair.
    #[default]
    PerAccountChannelPeer,
    /// One session per channel + sender (excludes `to` field).
    ///
    /// Used when agent-level isolation is provided by a per-agent
    /// [`SessionManager`] — the session key is `{channel}:{from}`, so
    /// different agents sharing the same channel naturally stay isolated
    /// without embedding `agent_id` in the key.
    PerChannelSender,
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
            DmScope::Main => format!("{}:{}", channel, message.to),
            DmScope::PerPeer => format!("{}:{}", message.from, message.to),
            DmScope::PerChannelPeer => {
                format!("{}:{}:{}", channel, message.from, message.to)
            }
            DmScope::PerAccountChannelPeer => {
                let acc = account_id.unwrap_or("default");
                format!("{}:{}:{}:{}", channel, message.from, message.to, acc)
            }
            DmScope::PerChannelSender => format!("{}:{}", channel, message.from),
        };
        let hash = Sha256::digest(routing_fields.as_bytes());
        format!("{}-{:x}", timestamp_ms, hash)
    }
}

// ---------------------------------------------------------------------------
// GatewayConfig
// ---------------------------------------------------------------------------

/// Gateway configuration.
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

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// Session — represents an active conversation.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel: String,
    pub created_at: i64,
    /// Nesting depth. 0 for root sessions, parent.depth + 1 for child sessions.
    pub depth: u32,
}

// ---------------------------------------------------------------------------
// HandleResult
// ---------------------------------------------------------------------------

/// Outcome of handling an inbound message.
#[derive(Debug)]
pub enum HandleResult {
    /// Message was queued (session busy).
    MessageQueued,
    /// An LLM call has been spawned and will run asynchronously.
    LlmStarted,
    /// An approval command was processed (approve/deny).
    ApprovalProcessed,
    /// A slash command was dispatched.
    SlashHandled,
}

// ---------------------------------------------------------------------------
// InboundRequest
// ---------------------------------------------------------------------------

/// An inbound message awaiting processing.
///
/// Stores the raw webhook payload so the consumer task can parse it
/// through the IM plugin _after_ entering the queue, matching the
/// design doc architecture where the queue sits before plugin parsing.
///
/// `peer_id` is stored separately for the busy-reply path (when the
/// queue is full, we need a target to reply to without parsing).
#[derive(Debug, Clone)]
pub struct InboundRequest {
    /// IM platform identifier (e.g. "feishu", "discord").
    pub platform: String,
    /// Raw webhook payload bytes.
    pub raw_payload: Vec<u8>,
    /// Peer / chat ID — used for busy-reply when the queue is full.
    pub peer_id: String,
}

// ---------------------------------------------------------------------------
// InboundChainInput
// ---------------------------------------------------------------------------

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
    /// Quoted/replied-to message content, if present.
    pub quoted_message: Option<String>,
}

// ---------------------------------------------------------------------------
// GatewayError
// ---------------------------------------------------------------------------

/// Errors returned by Gateway operations.
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
}
