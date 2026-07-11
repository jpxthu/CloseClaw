//! IM plugin trait and shared IM adapter types.
//!
//! Defines the [`IMPlugin`] trait interface that messaging platform
//! adapters implement, along with supporting types ([`AdapterError`],
//! [`NormalizedMessage`], [`RenderedOutput`]) shared across crates.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// MessageType
// ---------------------------------------------------------------------------

/// Normalized message type, matching the four variants defined in
/// `docs/design/common/shared-types.md`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum MessageType {
    /// Plain text message.
    #[default]
    Text,
    /// Image message.
    Image,
    /// File message.
    File,
    /// Audio message.
    Audio,
    /// Catch-all for platform-specific types not in the normalized set.
    Other(String),
}

impl Serialize for MessageType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            MessageType::Text => serializer.serialize_str("text"),
            MessageType::Image => serializer.serialize_str("image"),
            MessageType::File => serializer.serialize_str("file"),
            MessageType::Audio => serializer.serialize_str("audio"),
            MessageType::Other(s) => serializer.serialize_str(s),
        }
    }
}

impl<'de> Deserialize<'de> for MessageType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(MessageType::from(s.as_str()))
    }
}

impl From<&str> for MessageType {
    fn from(s: &str) -> Self {
        match s {
            "text" => MessageType::Text,
            "image" => MessageType::Image,
            "file" => MessageType::File,
            "audio" => MessageType::Audio,
            other => MessageType::Other(other.to_string()),
        }
    }
}

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

    /// Message type.
    ///
    /// Defaults to [`MessageType::Text`] when the platform does not specify a type.
    #[serde(default)]
    pub message_type: MessageType,

    /// Media attachment references (images, files, audio).
    #[serde(default)]
    pub media_refs: Vec<MediaRef>,

    /// Optional thread/topic ID. Used for threaded replies on platforms that
    /// support threads; does **not** participate in session key calculation.
    pub thread_id: Option<String>,

    /// Tenant/account identifier for multi-tenant session isolation.
    #[serde(default)]
    pub account_id: String,
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
// CardActionEvent
// ---------------------------------------------------------------------------

/// Platform-agnostic card-action interaction event.
///
/// Carries all information needed to inject a card action result into the
/// conversation as a tool-result payload, without going through the normal
/// inbound Processor Chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardActionEvent {
    /// Platform identifier, e.g. `"feishu"`.
    pub platform: String,
    /// Sender's platform-specific user ID.
    pub sender_id: String,
    /// The action value produced by the card interaction.
    pub action_value: String,
    /// Free-form metadata from the platform (e.g. card ID, action tag).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// Event timestamp as a Unix timestamp (milliseconds since epoch).
    pub timestamp: i64,
    /// Tenant/account identifier for multi-tenant session isolation.
    #[serde(default)]
    pub account_id: String,
}

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
// IMAdapter trait
// ---------------------------------------------------------------------------

/// Trait for sending messages through an IM adapter.
///
/// This is a simplified interface for components that only need to send
/// messages (e.g., notification during session restoration). For full
/// inbound/outbound handling, use [`IMPlugin`].
#[async_trait]
pub trait IMAdapter: Send + Sync {
    /// Send the rendered output to the target.
    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError>;
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
    /// empty content, unsupported message type, card-action event).
    /// Returns `Err` on parse failure.
    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError>;

    /// Parse an inbound webhook payload into a [`CardActionEvent`].
    ///
    /// Card-action events (button clicks, selector picks, etc.) bypass the
    /// normal inbound Processor Chain and are injected directly as tool-result
    /// payloads.
    ///
    /// Returns `Ok(None)` when the payload is not a card-action event.
    /// Returns `Err` on parse failure.
    ///
    /// The default implementation returns `Ok(None)`. Platforms that support
    /// card actions should override this.
    async fn parse_card_action(
        &self,
        _payload: &[u8],
    ) -> Result<Option<CardActionEvent>, AdapterError> {
        Ok(None)
    }

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

    /// Shut down the inbound side of the plugin (webhook listeners,
    /// websocket connections for receiving messages).
    ///
    /// Called during Phase 1 of daemon shutdown. The default implementation
    /// delegates to [`shutdown`](IMPlugin::shutdown) for backward compatibility.
    async fn shutdown_inbound(&self) -> Result<(), AdapterError> {
        self.shutdown().await
    }

    /// Shut down the outbound side of the plugin (message sending channels,
    /// API connections for delivering messages).
    ///
    /// Called during Phase 5 of daemon shutdown. The default implementation
    /// delegates to [`shutdown`](IMPlugin::shutdown) for backward compatibility.
    async fn shutdown_outbound(&self) -> Result<(), AdapterError> {
        self.shutdown().await
    }

    /// Render LLM content blocks into a platform-native output.
    ///
    /// The default implementation parses Text blocks via
    /// [`parse_content_segments`] to preserve fenced code blocks as single
    /// units with language annotations, then emits them in triple-backtick
    /// markdown format. This implements the "other" platform strategy:
    /// plain text with fenced code blocks preserved.
    ///
    /// Platforms should override for rich formatting (e.g. Feishu uses
    /// native markdown code blocks, CLI uses ANSI escape sequences).
    fn render(
        &self,
        content_blocks: &[crate::processor::ContentBlock],
        _dsl_result: Option<&crate::processor::DslParseResult>,
    ) -> RenderedOutput {
        let mut rendered = String::new();
        for block in content_blocks {
            if let crate::processor::ContentBlock::Text(text) = block {
                let segments = crate::code_block::parse_content_segments(text);
                for (i, segment) in segments.into_iter().enumerate() {
                    if i > 0 {
                        rendered.push('\n');
                    }
                    use crate::code_block::ContentSegment;
                    match segment {
                        ContentSegment::Markdown(line) => {
                            rendered.push_str(&line);
                        }
                        ContentSegment::Hr => {
                            rendered.push_str("---");
                        }
                        ContentSegment::CodeBlock { language, code } => {
                            if language.is_empty() {
                                rendered.push_str("```\n");
                            } else {
                                rendered.push_str(&format!("```{language}\n"));
                            }
                            rendered.push_str(&code);
                            rendered.push_str("\n```");
                        }
                    }
                }
            }
        }
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::String(rendered),
        }
    }

    /// Handle a streaming event from the LLM.
    ///
    /// Returns incremental output (text lines, render blocks) for the
    /// platform to deliver. The default implementation returns empty output.
    fn handle_stream_event(&self, _event: crate::processor::StreamEvent) -> StreamingOutput {
        StreamingOutput::default()
    }

    /// Flush any buffered streaming output.
    ///
    /// Called when the stream ends to drain any remaining buffered content.
    /// Returns empty output by default.
    fn flush_stream(&self) -> StreamingOutput {
        StreamingOutput::default()
    }

    /// Send a thinking/reasoning state indicator to the IM platform.
    ///
    /// Called by the Gateway when a Thinking `BlockStart` or `BlockEnd`
    /// event is detected during streaming. Platforms can override this
    /// to display a visual indicator (e.g. shimmer, typing animation)
    /// while the LLM is reasoning.
    ///
    /// - `active = true`: thinking started — show indicator
    /// - `active = false`: thinking ended — hide indicator
    ///
    /// The default implementation is a no-op. Platforms that support
    /// thinking indicators should override this method.
    fn send_thinking_indicator(&self, _active: bool) {}
}
