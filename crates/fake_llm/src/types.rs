//! Protocol-agnostic request and response types.
//!
//! These types are shared across protocol-specific parsing layers (OpenAI, Anthropic)
//! and passed to the scenario engine for decision-making.

use serde::{Deserialize, Serialize};

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
}

/// Scenario engine decision for a matched request.
///
/// This is the output of the scenario engine (Sequence 2) that tells the
/// delivery layer how to respond. In Step 1.1 this is a skeleton type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioDecision {
    /// The scenario name that matched this request.
    pub scenario: String,
    /// Whether to stream the response.
    pub stream: bool,
}

impl Default for ScenarioDecision {
    fn default() -> Self {
        Self {
            scenario: "default".to_string(),
            stream: false,
        }
    }
}
