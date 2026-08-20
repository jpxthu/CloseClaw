//! Scenario file types and protocol-agnostic decision types.
//!
//! Defines the JSON schema for scenario files (loaded by `loader.rs`)
//! and the types that flow between the scenario engine and the
//! protocol/delivery layers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{RequestFeatures, ScenarioDecision};

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
}

/// A single turn's response configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResponse {
    /// The response shape for this turn.
    pub response: ResponseShape,
    /// Optional artificial delay before delivering the response (milliseconds).
    #[serde(default)]
    pub delay: Option<u64>,
    /// Optional HTTP error injection. When present, the endpoint returns
    /// this error instead of a normal response.
    #[serde(default)]
    pub error: Option<HttpError>,
}

/// HTTP error to inject into a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpError {
    /// HTTP status code (e.g. 401, 429, 500).
    pub status: u16,
    /// Error message body.
    pub message: String,
}

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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseShape {
    /// Plain text content response.
    #[serde(rename = "text")]
    Text(TextResponse),

    /// Reasoning / thinking content (Phase 2+).
    #[serde(rename = "reasoning")]
    Reasoning,

    /// Tool call response (Phase 2+).
    #[serde(rename = "tool_call")]
    ToolCall,

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
    Unknown,
}

/// Plain text response content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextResponse {
    /// The text content to return.
    pub content: String,
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
    ) -> Self {
        Self {
            model,
            stream,
            max_tokens,
            temperature,
            messages,
            tools,
        }
    }
}

// ---------------------------------------------------------------------------
// ScenarioDecision extension
// ---------------------------------------------------------------------------

impl ScenarioDecision {
    /// Create a decision with response blocks (used by the scenario engine).
    pub fn with_blocks(scenario: String, stream: bool, blocks: Vec<ResponseBlock>) -> Self {
        Self {
            scenario,
            stream,
            response_blocks: blocks,
            http_error: None,
            delay: None,
            usage: None,
        }
    }

    /// Create an error decision (HTTP error injection).
    pub fn with_error(scenario: String, error: HttpError) -> Self {
        Self {
            scenario,
            stream: false,
            response_blocks: vec![],
            http_error: Some(error),
            delay: None,
            usage: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_scenario_file_minimal() {
        let json = r#"{
            "scenarios": [
                {
                    "name": "basic",
                    "turns": [
                        {
                            "response": {
                                "type": "text",
                                "content": "Hello!"
                            }
                        }
                    ]
                }
            ]
        }"#;
        let file: ScenarioFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.scenarios.len(), 1);
        assert_eq!(file.scenarios[0].name, "basic");
        assert!(file.scenarios[0].match_.is_none());
    }

    #[test]
    fn deserialize_scenario_file_with_match_condition() {
        let json = r#"{
            "scenarios": [
                {
                    "name": "gpt4-specific",
                    "match_": {
                        "model_id": "gpt-4o",
                        "message_contains": "hello"
                    },
                    "turns": [
                        {
                            "response": {
                                "type": "text",
                                "content": "Hi there!"
                            }
                        },
                        {
                            "response": {
                                "type": "text",
                                "content": "Still here!"
                            },
                            "delay": 100
                        }
                    ]
                }
            ]
        }"#;
        let file: ScenarioFile = serde_json::from_str(json).unwrap();
        let decl = &file.scenarios[0];
        let cond = decl.match_.as_ref().unwrap();
        assert_eq!(cond.model_id.as_deref(), Some("gpt-4o"));
        assert_eq!(cond.message_contains.as_deref(), Some("hello"));
        assert_eq!(decl.turns.len(), 2);
        assert_eq!(decl.turns[1].delay, Some(100));
    }

    #[test]
    fn deserialize_response_shape_variants() {
        // Text variant
        let json = r#"{"type": "text", "content": "hello"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Text(t) => assert_eq!(t.content, "hello"),
            _ => panic!("expected Text variant"),
        }

        // Error variant (placeholder)
        let json = r#"{"type": "error"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        assert!(matches!(shape, ResponseShape::Error));

        // Usage variant
        let json = r#"{"type": "usage", "prompt_tokens": 10, "completion_tokens": 20}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Usage(u) => {
                assert_eq!(u.prompt_tokens, Some(10));
                assert_eq!(u.completion_tokens, Some(20));
            }
            _ => panic!("expected Usage variant"),
        }

        // Unknown/placeholder variants
        let json = r#"{"type": "reasoning"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        assert!(matches!(shape, ResponseShape::Reasoning));

        let json = r#"{"type": "streaming"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        assert!(matches!(shape, ResponseShape::Streaming));
    }

    #[test]
    fn deserialize_http_error() {
        let json = r#"{"status": 429, "message": "rate limited"}"#;
        let err: HttpError = serde_json::from_str(json).unwrap();
        assert_eq!(err.status, 429);
        assert_eq!(err.message, "rate limited");
    }

    #[test]
    fn deserialize_turn_response_with_delay_and_error() {
        let json = r#"{
            "response": {"type": "text", "content": "ok"},
            "delay": 500,
            "error": {"status": 500, "message": "server error"}
        }"#;
        let turn: TurnResponse = serde_json::from_str(json).unwrap();
        assert_eq!(turn.delay, Some(500));
        assert_eq!(turn.error.as_ref().unwrap().status, 500);
    }

    #[test]
    fn deserialize_match_condition_defaults() {
        let json = r#"{}"#;
        let cond: MatchCondition = serde_json::from_str(json).unwrap();
        assert!(cond.model_id.is_none());
        assert!(cond.message_contains.is_none());
        assert!(cond.tool_name.is_none());
        assert!(cond.extra.is_none());
    }

    #[test]
    fn scenario_decision_default_has_empty_blocks() {
        let decision = ScenarioDecision::default();
        assert_eq!(decision.scenario, "default");
        assert!(decision.response_blocks.is_empty());
        assert!(decision.http_error.is_none());
        assert!(decision.delay.is_none());
        assert!(decision.usage.is_none());
    }

    #[test]
    fn usage_response_all_fields_optional() {
        let json = r#"{"type": "usage"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Usage(u) => {
                assert!(u.prompt_tokens.is_none());
                assert!(u.completion_tokens.is_none());
                assert!(u.reasoning_tokens.is_none());
                assert!(u.cache_hit_tokens.is_none());
                assert!(u.cache_write_tokens.is_none());
            }
            _ => panic!("expected Usage variant"),
        }
    }

    #[test]
    fn response_block_roundtrip() {
        let block = ResponseBlock {
            block_type: "text".to_string(),
            content: Some("Hello".to_string()),
            tool_name: None,
            tool_arguments: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: ResponseBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.block_type, "text");
        assert_eq!(parsed.content.as_deref(), Some("Hello"));
    }
}
