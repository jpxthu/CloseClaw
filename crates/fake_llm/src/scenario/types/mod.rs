//! Scenario file types and protocol-agnostic decision types.
//!
//! Defines the JSON schema for scenario files (loaded by `loader.rs`)
//! and the types that flow between the scenario engine and the
//! protocol/delivery layers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{ProtocolKind, RequestFeatures, ScenarioDecision};

mod response_shapes;
pub use response_shapes::*;

// ---------------------------------------------------------------------------
// Response or composite
// ---------------------------------------------------------------------------

/// A turn's response field that accepts either a single shape or an array.
///
/// Existing scenario files with a single shape object continue to work.
/// New files can use an array to combine multiple shapes in one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseOrComposite {
    /// Single response shape (backward-compatible default).
    Single(ResponseShape),
    /// Multiple response shapes combined in one turn.
    Multiple(Vec<ResponseShape>),
}

impl ResponseOrComposite {
    /// Flatten into a list of owned response shapes.
    pub fn to_shapes(&self) -> Vec<ResponseShape> {
        match self {
            ResponseOrComposite::Single(s) => vec![s.clone()],
            ResponseOrComposite::Multiple(v) => v.clone(),
        }
    }
}

impl Default for ResponseOrComposite {
    fn default() -> Self {
        ResponseOrComposite::Single(ResponseShape::Unknown)
    }
}

impl From<ResponseShape> for ResponseOrComposite {
    fn from(shape: ResponseShape) -> Self {
        ResponseOrComposite::Single(shape)
    }
}

// ---------------------------------------------------------------------------
// Scenario file types
// ---------------------------------------------------------------------------

/// Top-level structure of a scenario file.
///
/// Each file contains zero or more scenario declarations. The loader
/// merges declarations from multiple files into a single list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioFile {
    /// All scenario declarations in this file.
    pub scenarios: Vec<ScenarioDeclaration>,
}

/// A single model entry in the models list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Model ID (e.g. "gpt-4", "claude-3-opus-20240229").
    pub id: String,
    /// Owning organization (e.g. "openai", "anthropic").
    #[serde(default = "default_owned_by")]
    pub owned_by: String,
}

fn default_owned_by() -> String {
    "openai".to_string()
}

/// A single scenario declaration: matching condition + response sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioDeclaration {
    /// Human-readable scenario name (used in logs and error messages).
    pub name: String,
    /// Optional matching conditions. `None` means this is a fallback scenario.
    #[serde(default)]
    pub match_: Option<MatchCondition>,
    /// Ordered turn responses. The N-th request within a session returns
    /// the N-th turn (0-indexed). Exceeding this count is an error.
    pub turns: Vec<TurnResponse>,
    /// Optional model list declaration for `/v1/models` endpoint.
    /// When present, the models endpoint returns this list instead of
    /// the default placeholder list.
    #[serde(default)]
    pub models: Option<Vec<ModelEntry>>,
}

/// Conditions that determine whether a request matches this scenario.
///
/// All non-None fields must match for the scenario to be selected.
/// A `None` field is treated as "any" (not a constraint).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MatchCondition {
    /// Exact model ID match (e.g. `"gpt-4o"`).
    #[serde(default)]
    pub model_id: Option<String>,
    /// If set, at least one message in the request must contain this substring.
    #[serde(default)]
    pub message_contains: Option<String>,
    /// If set, the request must reference a tool with this name.
    #[serde(default)]
    pub tool_name: Option<String>,
    /// Extra key-value match conditions (future extensibility).
    #[serde(default)]
    pub extra: Option<HashMap<String, String>>,
    /// Request parameter match conditions (e.g. stream, max_tokens, temperature).
    ///
    /// Keys correspond to field names on [`RequestFeatures`]: `"stream"`,
    /// `"max_tokens"`, `"temperature"`. Values are JSON-typed to preserve
    /// the original types (bool / number). Unknown keys are silently ignored
    /// during matching.
    #[serde(default)]
    pub request_params: Option<HashMap<String, serde_json::Value>>,
}

/// A single turn's response configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResponse {
    /// The response shape for this turn.
    /// Optional: error-only turns may omit this field.
    ///
    /// When the `error` field is also present, `error` takes priority and
    /// this field is ignored (see [`ScenarioEngine::decide`]).
    #[serde(default)]
    pub response: ResponseOrComposite,
    /// Optional artificial delay before delivering the response (milliseconds).
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
    /// Optional HTTP error injection. When present, the endpoint returns
    /// this error instead of a normal response.
    ///
    /// When this field is `Some`, the error takes priority over the
    /// `response` field — [`ScenarioEngine::decide`] returns immediately
    /// with [`DecisionOutcome::Error`], and `response` is never evaluated.
    #[serde(default)]
    pub error: Option<HttpError>,
    /// Optional stream interrupt position. When set, the streaming response
    /// stops after sending this many events (0 = first event then disconnect).
    #[serde(default)]
    pub stream_interrupt_after: Option<usize>,
}

/// HTTP error to inject into a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpError {
    /// HTTP status code (e.g. 401, 429, 500).
    pub status: u16,
    /// Error message body.
    pub message: String,
    /// Optional Retry-After header value (seconds).
    #[serde(default)]
    pub retry_after: Option<u64>,
}

// ---------------------------------------------------------------------------
// Extended request features (added in Step 1.1)
// ---------------------------------------------------------------------------

/// Simplified message representation for scenario matching.
///
/// The protocol layer extracts message content strings from the
/// protocol-specific message format and passes them here. Only
/// content that could match `message_contains` conditions is included.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEntry {
    /// Message role (e.g. `"user"`, `"assistant"`, `"system"`).
    pub role: String,
    /// Message content text (concatenated if multipart).
    pub content: String,
}

// ---------------------------------------------------------------------------
// Extended decision types (added in Step 1.1)
// ---------------------------------------------------------------------------

/// A single protocol-agnostic content block in a response.
///
/// The protocol layer maps these into the appropriate format:
/// - OpenAI: `content` array with `type: "text"` blocks
/// - Anthropic: `content` array with `type: "text"` or `type: "tool_use"` blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseBlock {
    /// Block type hint for protocol serialization.
    pub block_type: String,
    /// Text content (for text blocks).
    #[serde(default)]
    pub content: Option<String>,
    /// Tool call name (for tool_call blocks).
    #[serde(default)]
    pub tool_name: Option<String>,
    /// Tool call arguments as JSON string (for tool_call blocks).
    #[serde(default)]
    pub tool_arguments: Option<String>,
    /// Reasoning text (for reasoning blocks).
    #[serde(default)]
    pub reasoning: Option<String>,
    /// Reasoning signature (for reasoning blocks).
    #[serde(default)]
    pub signature: Option<String>,
}

// ---------------------------------------------------------------------------
// Request features extension
// ---------------------------------------------------------------------------

impl RequestFeatures {
    /// Build `RequestFeatures` with message and tool fields for scenario matching.
    pub fn with_features(
        model: String,
        stream: bool,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
        messages: Vec<MessageEntry>,
        tools: Vec<String>,
        protocol: ProtocolKind,
    ) -> Self {
        Self {
            model,
            stream,
            max_tokens,
            temperature,
            messages,
            tools,
            protocol,
        }
    }
}

// ---------------------------------------------------------------------------
// ScenarioDecision extension
// ---------------------------------------------------------------------------

impl ScenarioDecision {
    /// Create a decision with response blocks (used by the scenario engine).
    pub fn with_blocks(
        model: String,
        scenario: String,
        stream: bool,
        blocks: Vec<ResponseBlock>,
    ) -> Self {
        Self {
            model,
            scenario,
            stream,
            response_blocks: blocks,
            http_error: None,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            segment_granularity: None,
            usage: None,
        }
    }

    /// Create an error decision (HTTP error injection).
    pub fn with_error(model: String, scenario: String, error: HttpError) -> Self {
        Self {
            model,
            scenario,
            stream: false,
            response_blocks: vec![],
            http_error: Some(error),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            segment_granularity: None,
            usage: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
