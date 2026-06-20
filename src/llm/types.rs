//! Core types for LLM multi-Provider architecture.
//!
//! This module defines the unified type layer used across all Providers and Protocols.

use std::fmt::{self, Display};
use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::session::persistence::ReasoningLevel;

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

/// A single content block in a unified response.
///
/// Uses `#[serde(tag = "type")]` for JSON representation where
/// each variant is distinguished by the `type` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "content", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Text content block with a string payload.
    Text(String),
    /// Thinking content block with a string payload.
    Thinking(String),
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

/// Unified token usage statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UnifiedUsage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,
    /// Number of tokens in the completion.
    pub completion_tokens: u32,
    /// Total tokens used (optional, some providers omit this).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    /// Number of tokens used for reasoning (if reported by provider).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
    /// Number of tokens read from cache (cache hit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Number of tokens written to cache (cache creation).
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

/// Streaming content delta for incremental updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    /// Text delta.
    Text { text: String },
    /// Thinking delta.
    Thinking { thinking: String },
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
        /// Index of the block in the content blocks list.
        index: usize,
        /// Type of the block being started.
        block_type: ContentBlockType,
    },
    /// An incremental delta for a content block.
    BlockDelta {
        /// Index of the block being updated.
        index: usize,
        /// The delta content.
        delta: ContentDelta,
    },
    /// A content block has finished.
    BlockEnd {
        /// Index of the block that ended.
        index: usize,
        /// Type of the block that ended.
        block_type: ContentBlockType,
    },
    /// The message stream has ended.
    MessageEnd {
        /// Token usage statistics (if available).
        usage: Option<UnifiedUsage>,
        /// Reason why the message finished.
        finish_reason: Option<String>,
    },
    /// An error occurred during streaming.
    Error {
        /// Error message.
        message: String,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Protocol-internal raw types (not exposed in public API)
// ─────────────────────────────────────────────────────────────────────────────

/// Raw content block used internally by Protocol implementations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "content", rename_all = "snake_case")]
pub enum RawContentBlock {
    /// Text content block with a string payload.
    Text(String),
    /// Thinking content block with a string payload.
    Thinking(String),
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
}

/// Raw token usage used internally by Protocol implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RawUsage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,
    /// Number of tokens in the completion.
    pub completion_tokens: u32,
    /// Total tokens used (optional, some providers omit this).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    /// Cache read (hit) tokens (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Cache write (creation) tokens (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

/// Internal response assembled by a Protocol from raw provider output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct InternalResponse {
    /// Ordered list of raw content blocks.
    pub content_blocks: Vec<RawContentBlock>,
    /// Token usage statistics.
    pub usage: RawUsage,
    /// Reason why the response finished (e.g., "stop", "length").
    pub finish_reason: Option<String>,
}

impl From<InternalResponse> for UnifiedResponse {
    fn from(resp: InternalResponse) -> Self {
        Self {
            content_blocks: resp.content_blocks.into_iter().map(Into::into).collect(),
            usage: resp.usage.into(),
            finish_reason: resp.finish_reason,
        }
    }
}

impl From<RawContentBlock> for ContentBlock {
    fn from(block: RawContentBlock) -> Self {
        match block {
            RawContentBlock::Text(s) => ContentBlock::Text(s),
            RawContentBlock::Thinking(s) => ContentBlock::Thinking(s),
            RawContentBlock::ToolUse { id, name, input } => {
                ContentBlock::ToolUse { id, name, input }
            }
            RawContentBlock::ToolResult {
                tool_call_id,
                content,
            } => ContentBlock::ToolResult {
                tool_call_id,
                content,
            },
        }
    }
}

impl From<RawUsage> for UnifiedUsage {
    fn from(raw: RawUsage) -> Self {
        Self {
            prompt_tokens: raw.prompt_tokens,
            completion_tokens: raw.completion_tokens,
            total_tokens: raw.total_tokens,
            reasoning_tokens: None,
            cache_read_tokens: raw.cache_read_tokens,
            cache_write_tokens: raw.cache_write_tokens,
        }
    }
}

/// Raw SSE chunk parsed by a Protocol from an SSE event stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RawSseChunk {
    /// SSE event type (e.g., "message", "error").
    pub event_type: String,
    /// SSE data field content.
    pub data: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Protocol identity and request types
// ─────────────────────────────────────────────────────────────────────────────

/// Protocol identifier (e.g., "openai", "anthropic").
///
/// This is a newtype wrapper around `String` used to identify which
/// protocol a `ChatProtocol` implementation targets.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProtocolId(String);

impl ProtocolId {
    /// Creates a new `ProtocolId` from a string value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Returns the underlying protocol identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ProtocolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ProtocolId {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

impl From<String> for ProtocolId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// A structured system prompt block with cache control metadata.
///
/// Used by `CacheAdapter` implementations to produce provider-specific
/// system block representations (e.g., Anthropic cache_control markers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBlock {
    /// The text content of this system block.
    pub text: String,
    /// Whether this block should be marked as cacheable.
    pub cache: bool,
}

/// A single message in an `InternalRequest`.
///
/// Contains the role and content of a chat message used internally
/// by Protocol implementations when building provider requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalMessage {
    /// The role of the message sender (e.g., "user", "assistant").
    pub role: String,
    /// The content of the message.
    pub content: String,
}

/// Internal request structure used by `ChatProtocol` implementations.
///
/// This is the protocol-level representation of a chat completion request,
/// distinct from any provider-specific request types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalRequest {
    /// The model identifier to use for this request.
    pub model: String,
    /// Ordered list of chat messages.
    pub messages: Vec<InternalMessage>,
    /// Sampling temperature (default 0.0).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Maximum number of tokens to generate (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Whether to stream responses (default false).
    #[serde(default)]
    pub stream: bool,
    /// Additional provider-specific parameters.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    /// Static system prompt content (cacheable portion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_static: Option<String>,
    /// Dynamic system prompt content (non-cacheable portion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_dynamic: Option<String>,
    /// Structured system blocks produced by a `CacheAdapter`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_blocks: Option<Vec<SystemBlock>>,
    /// Session identifier used for provider-level cache keys (e.g., Kimi prompt_cache_key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Reasoning depth level for the request.
    /// Each protocol maps this to its native parameter (e.g., OpenAI `reasoning_effort`).
    /// Skipped during serialization to avoid leaking to API payloads.
    #[serde(skip)]
    pub reasoning_level: ReasoningLevel,
    /// Current turn count within the session, used for API metadata.
    /// Skipped during serialization when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<u32>,
}

fn default_temperature() -> f32 {
    0.0
}

// ─────────────────────────────────────────────────────────────────────────────
// SSE state machine for stream parsing
// ─────────────────────────────────────────────────────────────────────────────

/// State machine for parsing SSE event streams into structured stream events.
///
/// Tracks the current state during SSE parsing to correctly assemble
/// incremental deltas into complete content blocks.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
pub struct SseStateMachine {
    /// Index of the currently active content block being parsed.
    pub current_block_index: Option<usize>,
    /// Type of the currently active content block.
    pub current_block_type: Option<ContentBlockType>,
    /// Accumulated thinking content for the current thinking block.
    pub pending_thinking: String,
    /// Accumulated signature/tool call id for the current tool use block.
    pub pending_signature: String,
}

impl SseStateMachine {
    /// Creates a new SSE state machine in its initial state.
    pub fn new() -> Self {
        Self {
            current_block_index: None,
            current_block_type: None,
            pending_thinking: String::new(),
            pending_signature: String::new(),
        }
    }
}

impl Default for SseStateMachine {
    fn default() -> Self {
        Self::new()
    }
}
