//! Protocol-agnostic request and response types.
//!
//! These types are shared across protocol-specific parsing layers (OpenAI, Anthropic)
//! and passed to the scenario engine for decision-making.

use serde::{Deserialize, Serialize};

use crate::scenario::types::{HttpError, MessageEntry, ResponseBlock, UsageResponse};

// ---------------------------------------------------------------------------
// Shared extraction helpers
// ---------------------------------------------------------------------------

/// Extract text content from a JSON message value.
///
/// Handles both string content and array-of-parts content (multimodal).
/// Returns an empty string for null, numbers, booleans, or objects.
pub fn extract_text_from_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Extract tool name strings from a list of tool definition JSON values.
///
/// Handles both OpenAI format (`tools[].function.name`) and Anthropic
/// format (`tools[].name`).
pub fn extract_tool_names(tools: &[serde_json::Value]) -> Vec<String> {
    tools
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| t.get("name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect()
}

/// Protocol-agnostic request features extracted from incoming LLM requests.
///
/// Once parsed from either OpenAI or Anthropic protocol format, all downstream
/// logic (scenario matching, delivery) operates on this type exclusively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFeatures {
    /// The model ID requested by the client (e.g. "gpt-4", "claude-3-opus").
    pub model: String,
    /// Whether the client requested streaming.
    pub stream: bool,
    /// Maximum tokens to generate (if specified).
    pub max_tokens: Option<u32>,
    /// Temperature parameter (if specified).
    pub temperature: Option<f32>,
    /// Simplified message history for scenario matching.
    /// Protocol layer extracts content from the native message format.
    #[serde(default)]
    pub messages: Vec<MessageEntry>,
    /// Tool names referenced in the request.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// Scenario engine decision for a matched request.
///
/// This is the output of the scenario engine (Sequence 2) that tells the
/// delivery layer how to respond.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioDecision {
    /// The model ID from the original request (echoed back in responses).
    pub model: String,
    /// The scenario name that matched this request.
    pub scenario: String,
    /// Whether to stream the response.
    pub stream: bool,
    /// Protocol-agnostic content blocks for the response.
    #[serde(default)]
    pub response_blocks: Vec<ResponseBlock>,
    /// Optional HTTP error injection (overrides response_blocks).
    #[serde(default)]
    pub http_error: Option<HttpError>,
    /// Optional artificial delay before delivery (milliseconds).
    /// This is the overall delay applied to the entire response.
    #[serde(default)]
    pub delay: Option<u64>,
    /// Optional delay before the first token is emitted (milliseconds).
    /// When set, this delay is applied before any streaming content begins.
    #[serde(default)]
    pub first_token_delay: Option<u64>,
    /// Optional delay between each streaming segment (milliseconds).
    /// Applied between consecutive content deltas in streaming mode.
    #[serde(default)]
    pub segment_delay: Option<u64>,
    /// Optional stream interrupt position. When set, the streaming response
    /// stops after sending this many events (0 = first event then disconnect).
    #[serde(default)]
    pub stream_interrupt_after: Option<usize>,
    /// Optional token usage report.
    #[serde(default)]
    pub usage: Option<UsageResponse>,
}

impl Default for ScenarioDecision {
    fn default() -> Self {
        Self {
            model: "default".to_string(),
            scenario: "default".to_string(),
            stream: false,
            response_blocks: vec![],
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            usage: None,
        }
    }
}
