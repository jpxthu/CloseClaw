//! Processor chain types and trait.
//!
//! Defines the shared types ([`ProcessedMessage`], [`RawMessage`],
//! [`DslParseResult`], [`ProcessError`]) and the [`ProcessorChain`]
//! trait used by the gateway to run inbound/outbound message processing.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ContentBlock (shared definition)
// ---------------------------------------------------------------------------

/// A single content block in a unified response.
///
/// This is the shared definition used across workspace crates.
/// The LLM crate re-exports this type for backward compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "content", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Text content block with a string payload.
    Text(String),
    /// Thinking content block with a reasoning trace and optional signature.
    Thinking {
        /// The thinking/reasoning content.
        thinking: String,
        /// Optional signature for traceability (e.g., Anthropic thinking signature).
        signature: Option<String>,
    },
    /// Tool use invocation block.
    ToolUse {
        /// Tool call identifier.
        id: String,
        /// Tool name being invoked.
        name: String,
        /// Tool input/arguments.
        input: String,
    },
    /// Tool result block.
    ToolResult {
        /// The tool call ID this result corresponds to.
        tool_call_id: String,
        /// Result content returned by the tool.
        content: String,
    },
    /// Image content block (file name or identifier).
    Image(String),
    /// Audio content block (file name or identifier).
    Audio(String),
    /// File content block (file name or identifier).
    File(String),
}

/// Content block type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentBlockType {
    /// Text content block.
    Text,
    /// Thinking content block (reasoning trace).
    Thinking,
    /// Tool use invocation block.
    ToolUse,
}

/// Content delta for streaming responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    /// Text delta.
    Text { text: String },
    /// Thinking delta.
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
    /// Tool use call ID delta.
    ToolUseId { id: String },
    /// Tool use name delta.
    ToolUseName { name: String },
    /// Tool use input chunk delta.
    ToolUseInputChunk { input: String },
}

/// Stream event emitted during streaming responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// A new content block has started.
    BlockStart {
        index: usize,
        block_type: ContentBlockType,
    },
    /// An incremental delta for a content block.
    BlockDelta { index: usize, delta: ContentDelta },
    /// A content block has finished.
    BlockEnd {
        index: usize,
        block_type: ContentBlockType,
    },
    /// The message stream has ended.
    MessageEnd {
        usage: Option<UnifiedUsage>,
        finish_reason: Option<String>,
    },
    /// An error occurred during streaming.
    Error { message: String },
}

/// Unified token usage statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UnifiedUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

/// Unified response structure returned by all LLM providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UnifiedResponse {
    /// Ordered list of content blocks.
    pub content_blocks: Vec<ContentBlock>,
    /// Token usage statistics.
    pub usage: UnifiedUsage,
    /// Reason why the response finished (e.g., "stop", "length").
    pub finish_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// RawMessage
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// ProcessedMessage
// ---------------------------------------------------------------------------

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
    /// Structured content blocks carried from the outbound chain.
    #[serde(default)]
    pub content_blocks: Vec<ContentBlock>,
}

impl ProcessedMessage {
    /// Creates a default processed message from raw content.
    pub fn from_raw_content(content: String) -> Self {
        Self {
            content,
            metadata: serde_json::Map::new(),
            suppress: false,
            content_blocks: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// DslParseResult
// ---------------------------------------------------------------------------

/// A parsed DSL instruction extracted from markdown.
///
/// Flat structure: `instruction_type` identifies the kind of instruction
/// (e.g. `"button"`, `"selector"`), and `params` holds key-value pairs
/// parsed from the DSL line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DslInstruction {
    /// Instruction type identifier (e.g. `"button"`, `"selector"`).
    pub instruction_type: String,
    /// Parsed key-value parameters from the DSL line.
    pub params: HashMap<String, String>,
}

/// Result of parsing a markdown string for DSL instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DslParseResult {
    /// Extracted DSL instructions in the order they appear.
    pub instructions: Vec<DslInstruction>,
}

// ---------------------------------------------------------------------------
// ProcessError
// ---------------------------------------------------------------------------

/// Errors raised during message processing.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// A processor in the chain returned an error.
    #[error("processor `{name}` failed")]
    ProcessorFailed {
        name: String,
        #[source]
        source: anyhow::Error,
    },

    /// The inbound/outbound message was malformed.
    #[error("invalid message: {0}")]
    InvalidMessage(String),

    /// The processor chain itself failed (e.g., empty chain misconfiguration).
    #[error("chain failed: {0}")]
    ChainFailed(String),
}

impl ProcessError {
    /// Constructs a `ProcessorFailed` error.
    #[inline]
    pub fn processor_failed(name: impl Into<String>, source: impl std::fmt::Display) -> Self {
        Self::ProcessorFailed {
            name: name.into(),
            source: anyhow::Error::msg(source.to_string()),
        }
    }

    /// Constructs an `InvalidMessage` error.
    #[inline]
    pub fn invalid_message(msg: impl Into<String>) -> Self {
        Self::InvalidMessage(msg.into())
    }

    /// Constructs a `ChainFailed` error.
    #[inline]
    pub fn chain_failed(msg: impl Into<String>) -> Self {
        Self::ChainFailed(msg.into())
    }
}

// ---------------------------------------------------------------------------
// ProcessorChain trait
// ---------------------------------------------------------------------------

/// Trait for running inbound/outbound message processing chains.
///
/// Implemented by `ProcessorRegistry` in the processor_chain crate;
/// used by the gateway to process messages without a direct dependency
/// on the processor chain internals.
#[async_trait]
pub trait ProcessorChain: Send + Sync {
    /// Process an inbound raw message through the inbound chain.
    async fn process_inbound(&self, raw: RawMessage) -> Result<ProcessedMessage, ProcessError>;

    /// Process an outbound message through the outbound chain.
    async fn process_outbound(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, ProcessError>;

    /// Number of inbound processors in the chain.
    fn inbound_len(&self) -> usize {
        0
    }

    /// Number of outbound processors in the chain.
    fn outbound_len(&self) -> usize {
        0
    }
}
