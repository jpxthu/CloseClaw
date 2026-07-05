//! Shared LLM request/response types.
//!
//! These pure data structures were moved from `closeclaw-llm` to
//! `closeclaw-common` so that cross-layer traits (e.g., [`LlmCaller`])
//! can reference them without creating circular dependencies.

use serde::{Deserialize, Serialize};

use crate::ReasoningLevel;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// A single message in an [`InternalRequest`].
///
/// Contains the role and content of a chat message used internally
/// by Protocol implementations when building provider requests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalMessage {
    /// The role of the message sender (e.g., "user", "assistant").
    pub role: String,
    /// The content of the message.
    pub content: String,
    /// Optional tool call ID for tool result messages.
    /// When present, this message represents a tool result that should be
    /// serialized in provider-native format by the protocol layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

// ---------------------------------------------------------------------------
// System / tool types
// ---------------------------------------------------------------------------

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

/// A tool definition passed via the API `tools` parameter.
///
/// Used by `CacheAdapter` to mark tool schemas as cacheable.
/// When serialized to an Anthropic-compatible payload, the `name`,
/// `description`, and `input_schema` fields map directly to the
/// Anthropic `tools` array format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The name of the tool.
    pub name: String,
    /// A human-readable description of what the tool does.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The JSON Schema describing the tool's input parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// Whether this tool's schema should be marked as cacheable.
    pub cache: bool,
}

// ---------------------------------------------------------------------------
// Internal request
// ---------------------------------------------------------------------------

fn default_temperature() -> f32 {
    0.0
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
    /// Tool definitions passed via the API `tools` parameter.
    /// When present, the cache adapter marks each tool's schema as cacheable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
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
