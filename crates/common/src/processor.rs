//! Processor chain types and trait.
//!
//! Defines the shared types ([`ProcessedMessage`], [`DslParseResult`],
//! [`ProcessError`]) and the [`ProcessorChain`]
//! trait used by the gateway to run inbound/outbound message processing.
//!
//! The inbound chain accepts [`NormalizedMessage`](crate::im_plugin::NormalizedMessage)
//! directly — no intermediate `RawMessage` wrapper.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::im_plugin::NormalizedMessage;

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
    /// Image reference block with a resource identifier and access URL.
    Image {
        /// Resource identifier (e.g. file name or key).
        name: String,
        /// Access URL for the image resource.
        url: String,
    },
    /// Audio reference block with a resource identifier and access URL.
    Audio {
        /// Resource identifier (e.g. file name or key).
        name: String,
        /// Access URL for the audio resource.
        url: String,
    },
    /// File reference block with a resource identifier and access URL.
    File {
        /// Resource identifier (e.g. file name or key).
        name: String,
        /// Access URL for the file resource.
        url: String,
    },
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
    /// Tool result block.
    ToolResult,
    /// Image content block.
    Image,
    /// Audio content block.
    Audio,
    /// File content block.
    File,
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
    /// Tool result text delta.
    ToolResultText { text: String },
    /// Image reference delta (resource identifier + URL).
    ImageRef { name: String, url: String },
    /// Audio reference delta (resource identifier + URL).
    AudioRef { name: String, url: String },
    /// File reference delta (resource identifier + URL).
    FileRef { name: String, url: String },
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
// ProcessedMessage
// ---------------------------------------------------------------------------

/// The result of running the full processor chain.
///
/// Produced after either the inbound or outbound chain completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedMessage {
    /// Structured content blocks carried from the chain.
    #[serde(default)]
    pub content_blocks: Vec<ContentBlock>,
    /// Metadata accumulated across all processors.
    pub metadata: HashMap<String, String>,
}

impl ProcessedMessage {
    /// Creates a default processed message from raw content.
    pub fn from_raw_content(content: String) -> Self {
        Self {
            content_blocks: vec![ContentBlock::Text(content)],
            metadata: HashMap::new(),
        }
    }

    /// Returns the first text content block's text, if present.
    pub fn text_content(&self) -> Option<&str> {
        self.content_blocks.iter().find_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
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
    /// Process an inbound normalized message through the inbound chain.
    async fn process_inbound(
        &self,
        msg: NormalizedMessage,
    ) -> Result<ProcessedMessage, ProcessError>;

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
