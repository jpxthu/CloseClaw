use serde::{Deserialize, Serialize};

/// Normalized inbound message produced by IM platform adapters.
///
/// This is the unified intermediate structure across all messaging platforms,
/// shielding platform-specific differences from the Processor Chain and Gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedMessage {
    /// Platform identifier, e.g. `"feishu"`, `"discord"`.
    pub platform: String,

    /// Sender's platform-specific user ID.
    pub sender_id: String,

    /// Peer ID — a `chat_id` for group chats, or the other party's user ID for
    /// private chats.
    pub peer_id: String,

    /// Message text content.
    pub content: String,

    /// Message send time as a Unix timestamp (seconds since epoch).
    pub timestamp: i64,

    /// Optional thread/topic ID. Used for定向 replies on platforms that support
    /// threads; does **not** participate in session key calculation.
    pub thread_id: Option<String>,

    /// Optional tenant/account identifier for multi-tenant session isolation.
    pub account_id: Option<String>,
}
