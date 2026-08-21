//! Protocol-agnostic response shape types.
//!
//! Defines the seven categories of response shapes that the protocol
//! layer serializes into OpenAI or Anthropic format.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

/// Seven categories of protocol-agnostic response shapes.
///
/// The protocol layer serializes these into OpenAI or Anthropic format
/// per `docs/design/llm/protocol-mapping.md`.
///
/// Phase 1 implements Text, Error, and Usage. Remaining variants are
/// placeholders for future phases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseShape {
    /// Plain text content response.
    #[serde(rename = "text")]
    Text(TextResponse),

    /// Reasoning / thinking content.
    #[serde(rename = "reasoning")]
    Reasoning(ReasoningResponse),

    /// Tool call response.
    #[serde(rename = "tool_call")]
    ToolCall(ToolCallResponse),

    /// Streaming response (Phase 2+).
    #[serde(rename = "streaming")]
    Streaming,

    /// Error response — HTTP status error injection.
    #[serde(rename = "error")]
    Error,

    /// Delay-only response (Phase 2+).
    #[serde(rename = "delay")]
    Delay,

    /// Token usage report (Phase 2+).
    #[serde(rename = "usage")]
    Usage(UsageResponse),

    /// Catch-all for unimplemented variants (serde default).
    #[serde(other)]
    #[default]
    Unknown,
}

/// Plain text response content.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextResponse {
    /// The text content to return.
    #[serde(default)]
    pub content: String,
    /// Optional token usage report.
    #[serde(default)]
    pub usage: Option<UsageResponse>,
}

/// Reasoning intensity level controlling the length of generated
/// reasoning content. Low produces short reasoning, Medium is the
/// default, and High produces lengthy reasoning.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReasoningIntensity {
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    #[default]
    Medium,
    #[serde(rename = "high")]
    High,
}

/// Reasoning / thinking response content.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReasoningResponse {
    /// The visible text content.
    #[serde(default)]
    pub content: String,
    /// The hidden reasoning text.
    #[serde(default)]
    pub reasoning: String,
    /// Optional reasoning signature for verification.
    #[serde(default)]
    pub signature: Option<String>,
    /// Optional token usage report.
    #[serde(default)]
    pub usage: Option<UsageResponse>,
    /// Reasoning intensity level controlling the length of generated
    /// reasoning content. Defaults to Medium.
    #[serde(default)]
    pub intensity: ReasoningIntensity,
}

/// A single tool call entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEntry {
    /// The tool function name.
    pub name: String,
    /// The arguments as a JSON string.
    pub arguments: String,
}

/// Tool call response containing one or more calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCallResponse {
    /// The list of tool calls to execute.
    #[serde(default)]
    pub calls: Vec<ToolCallEntry>,
    /// Optional token usage report.
    #[serde(default)]
    pub usage: Option<UsageResponse>,
}

/// Token usage breakdown.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageResponse {
    /// Number of prompt tokens.
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    /// Number of completion tokens.
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    /// Number of reasoning tokens (if applicable).
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
    /// Cache hit tokens.
    #[serde(default)]
    pub cache_hit_tokens: Option<u32>,
    /// Cache write tokens.
    #[serde(default)]
    pub cache_write_tokens: Option<u32>,
    /// When true, this provider does not return cache fields in
    /// responses. Auto-simulation is skipped (but the state machine
    /// still tracks prefix fingerprints internally).
    #[serde(default)]
    pub cache_fields_missing: bool,
}
