//! IM plugin trait and shared IM adapter types.
//!
//! Defines the [`IMPlugin`] trait interface that messaging platform
//! adapters implement, along with supporting types ([`AdapterError`],
//! [`NormalizedMessage`], [`RenderedOutput`]) shared across crates.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AdapterError
// ---------------------------------------------------------------------------

/// Common error type for IM adapter operations.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Unsupported operation")]
    UnsupportedOperation,
}

// ---------------------------------------------------------------------------
// MediaRef
// ---------------------------------------------------------------------------

/// Reference to a media attachment (image, file, audio) in a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    /// Platform-specific media key for downloading the resource.
    pub key: String,
    /// URL pointing to the media resource.
    pub url: String,
}

// ---------------------------------------------------------------------------
// QuotedMessage
// ---------------------------------------------------------------------------

/// Quoted/replied-to message embedded in an inbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotedMessage {
    /// Text content of the quoted message.
    pub content: String,
    /// Sender ID of the quoted message, if available.
    pub sender_id: Option<String>,
}

// ---------------------------------------------------------------------------
// NormalizedMessage
// ---------------------------------------------------------------------------

/// Platform-agnostic inbound message produced by an IM adapter.
///
/// Shields platform-specific differences from the Processor Chain and
/// Gateway, providing a uniform interface for downstream processing.
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

    /// Message send time as a Unix timestamp (milliseconds since epoch).
    pub timestamp: i64,

    /// Message type (`"text"`, `"image"`, `"file"`, `"audio"`, etc.).
    ///
    /// Defaults to `"text"` when the platform does not specify a type.
    #[serde(default = "default_message_type")]
    pub message_type: String,

    /// Media attachment references (images, files, audio).
    #[serde(default)]
    pub media_refs: Vec<MediaRef>,

    /// Quoted/replied-to message, if present. At most one level of nesting.
    pub quoted_message: Option<QuotedMessage>,

    /// Optional thread/topic ID. Used for threaded replies on platforms that
    /// support threads; does **not** participate in session key calculation.
    pub thread_id: Option<String>,

    /// Optional tenant/account identifier for multi-tenant session isolation.
    pub account_id: Option<String>,

    /// Whether this message is a card action (e.g. button click).
    ///
    /// `Some(true)` when the inbound event is a card action trigger;
    /// `None` (default) for regular text messages.
    #[serde(default)]
    pub card_action: Option<bool>,
}

fn default_message_type() -> String {
    "text".to_string()
}

// ---------------------------------------------------------------------------
// RenderedOutput
// ---------------------------------------------------------------------------

/// Output produced by rendering LLM content for a specific platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedOutput {
    /// Message type, e.g. `"text"` or `"interactive"`.
    pub msg_type: String,
    /// Platform-specific payload JSON.
    pub payload: serde_json::Value,
}

use crate::processor::ContentBlock;

// ---------------------------------------------------------------------------
// StreamingOutput
// ---------------------------------------------------------------------------

/// Incremental output from streaming LLM responses.
///
/// Carries completed text lines and non-text content blocks
/// produced during a streaming response batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamingOutput {
    /// Completed text lines emitted by the line buffer.
    pub text_messages: Vec<String>,
    /// Non-Text content blocks completed in this batch.
    pub render_blocks: Vec<ContentBlock>,
}

// ---------------------------------------------------------------------------
// IMPlugin trait
// ---------------------------------------------------------------------------

/// Unified plugin trait for messaging platforms.
///
/// Each platform (Feishu, Discord, etc.) implements this trait to provide:
///
/// - **Inbound**: Parse a raw webhook payload into a [`NormalizedMessage`].
/// - **Outbound sending**: Deliver rendered output to the platform.
/// - **Lifecycle**: Initialize on startup and shut down on daemon exit.
///
/// Gateway maintains a `platform → IMPlugin` registry and routes messages
/// through the matching plugin without caring about platform internals.
#[async_trait]
pub trait IMPlugin: Send + Sync {
    /// Returns the platform identifier, e.g. `"feishu"` or `"discord"`.
    fn platform(&self) -> &str;

    /// Parse an inbound webhook payload into a [`NormalizedMessage`].
    ///
    /// Returns `Ok(None)` when the payload should be silently ignored (e.g.
    /// empty content, unsupported message type). Returns `Err` on parse failure.
    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError>;

    /// Validate the webhook signature.
    ///
    /// The default implementation always returns `true`. Platforms that require
    /// signature verification should override this.
    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        true
    }

    /// Send the rendered output to the platform.
    ///
    /// `peer_id` identifies the target chat or user. `thread_id` optionally
    /// directs the message into a specific thread/topic.
    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError>;

    /// Clean platform-native text by removing platform-specific markers.
    ///
    /// Receives raw platform text (e.g. Feishu `<at>` tags, Discord mentions)
    /// and returns cleaned plain text. Called by the Processor Chain
    /// ContentNormalizer.
    ///
    /// The default implementation passes text through unchanged.
    fn clean_content(&self, raw: &str) -> String {
        raw.to_string()
    }

    /// Initialize the plugin on startup (connect pool, token, etc.).
    ///
    /// Plugins that do not need initialization can use the default no-op.
    async fn init(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Shut down the plugin and release resources.
    ///
    /// Called during daemon shutdown to clean up connections, caches, etc.
    /// Plugins that do not need cleanup can use the default no-op.
    async fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}
