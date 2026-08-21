//! Scenario file types and protocol-agnostic decision types.
//!
//! Defines the JSON schema for scenario files (loaded by `loader.rs`)
//! and the types that flow between the scenario engine and the
//! protocol/delivery layers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{RequestFeatures, ScenarioDecision};

mod response_shapes;
pub use response_shapes::*;

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
}

/// A single turn's response configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResponse {
    /// The response shape for this turn.
    /// Optional: error-only turns may omit this field.
    #[serde(default)]
    pub response: ResponseShape,
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

        // Reasoning variant with data
        let json = r#"{"type": "reasoning", "content": "text", "reasoning": "think"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Reasoning(r) => {
                assert_eq!(r.content, "text");
                assert_eq!(r.reasoning, "think");
            }
            _ => panic!("expected Reasoning variant"),
        }

        // Streaming variant (placeholder)
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
        assert!(err.retry_after.is_none());
    }

    #[test]
    fn deserialize_http_error_with_retry_after() {
        let json = r#"{"status": 429, "message": "rate limited", "retry_after": 60}"#;
        let err: HttpError = serde_json::from_str(json).unwrap();
        assert_eq!(err.status, 429);
        assert_eq!(err.message, "rate limited");
        assert_eq!(err.retry_after, Some(60));
    }

    #[test]
    fn deserialize_http_error_retry_after_default_none() {
        let json = r#"{"status": 500, "message": "error"}"#;
        let err: HttpError = serde_json::from_str(json).unwrap();
        assert!(err.retry_after.is_none());
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
    fn usage_response_cache_fields_missing_deserialize() {
        let json = r#"{"type": "usage", "prompt_tokens": 10, "completion_tokens": 20, "cache_fields_missing": true}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Usage(u) => {
                assert!(
                    u.cache_fields_missing,
                    "cache_fields_missing should be true"
                );
                assert_eq!(u.prompt_tokens, Some(10));
                assert_eq!(u.completion_tokens, Some(20));
            }
            _ => panic!("expected Usage variant"),
        }
    }

    #[test]
    fn usage_response_cache_fields_missing_default_false() {
        let json = r#"{"type": "usage", "prompt_tokens": 5}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Usage(u) => {
                assert!(
                    !u.cache_fields_missing,
                    "cache_fields_missing should default to false"
                );
            }
            _ => panic!("expected Usage variant"),
        }
    }

    #[test]
    fn text_response_with_cache_fields_missing_usage() {
        let json = r#"{"type": "text", "content": "hello", "usage": {"prompt_tokens": 10, "cache_fields_missing": true}}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Text(t) => {
                let u = t.usage.as_ref().unwrap();
                assert!(u.cache_fields_missing);
                assert_eq!(u.prompt_tokens, Some(10));
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn response_block_roundtrip() {
        let block = ResponseBlock {
            block_type: "text".to_string(),
            content: Some("Hello".to_string()),
            tool_name: None,
            tool_arguments: None,
            reasoning: None,
            signature: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: ResponseBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.block_type, "text");
        assert_eq!(parsed.content.as_deref(), Some("Hello"));
    }

    #[test]
    fn response_block_with_tool_fields() {
        let block = ResponseBlock {
            block_type: "tool_call".to_string(),
            content: None,
            tool_name: Some("get_weather".to_string()),
            tool_arguments: Some("{\"city\": \"Beijing\"}".to_string()),
            reasoning: None,
            signature: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: ResponseBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_name.as_deref(), Some("get_weather"));
        assert!(parsed.content.is_none());
    }

    #[test]
    fn message_entry_roundtrip() {
        let entry = MessageEntry {
            role: "user".to_string(),
            content: "Hello world".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MessageEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, "user");
        assert_eq!(parsed.content, "Hello world");
    }

    #[test]
    fn scenario_file_roundtrip() {
        let file = ScenarioFile {
            scenarios: vec![ScenarioDeclaration {
                name: "roundtrip-test".to_string(),
                match_: Some(MatchCondition {
                    model_id: Some("gpt-4o".to_string()),
                    message_contains: Some("hello".to_string()),
                    tool_name: None,
                    extra: None,
                }),
                turns: vec![TurnResponse {
                    response: ResponseShape::Text(TextResponse {
                        content: "hi".to_string(),
                        usage: None,
                    }),
                    delay: Some(100),
                    first_token_delay: None,
                    segment_delay: None,
                    error: None,
                    stream_interrupt_after: None,
                }],
                models: None,
            }],
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: ScenarioFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.scenarios.len(), 1);
        assert_eq!(parsed.scenarios[0].name, "roundtrip-test");
        assert_eq!(parsed.scenarios[0].turns[0].delay, Some(100));
    }

    #[test]
    fn deserialize_match_condition_all_fields() {
        let json = r#"{
            "model_id": "gpt-4o",
            "message_contains": "calculate",
            "tool_name": "search",
            "extra": {"env": "test"}
        }"#;
        let cond: MatchCondition = serde_json::from_str(json).unwrap();
        assert_eq!(cond.model_id.as_deref(), Some("gpt-4o"));
        assert_eq!(cond.message_contains.as_deref(), Some("calculate"));
        assert_eq!(cond.tool_name.as_deref(), Some("search"));
        assert_eq!(
            cond.extra.as_ref().unwrap().get("env"),
            Some(&"test".to_string())
        );
    }

    #[test]
    fn deserialize_match_condition_tool_name_only() {
        let json = r#"{"tool_name": "code_exec"}"#;
        let cond: MatchCondition = serde_json::from_str(json).unwrap();
        assert!(cond.model_id.is_none());
        assert!(cond.message_contains.is_none());
        assert_eq!(cond.tool_name.as_deref(), Some("code_exec"));
    }

    #[test]
    fn deserialize_response_shape_reasoning_with_data() {
        let json = r#"{"type": "reasoning", "content": "The answer is 42.", "reasoning": "Let me think..."}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Reasoning(r) => {
                assert_eq!(r.content, "The answer is 42.");
                assert_eq!(r.reasoning, "Let me think...");
                assert!(r.signature.is_none());
            }
            _ => panic!("expected Reasoning variant"),
        }
    }

    #[test]
    fn deserialize_response_shape_reasoning_with_signature() {
        let json = r#"{"type": "reasoning", "content": "ok", "reasoning": "because", "signature": "sig123"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Reasoning(r) => {
                assert_eq!(r.signature.as_deref(), Some("sig123"));
            }
            _ => panic!("expected Reasoning variant"),
        }
    }

    #[test]
    fn deserialize_response_shape_tool_call() {
        let json = serde_json::json!({
            "type": "tool_call",
            "calls": [{"name": "get_weather", "arguments": "{}"}]
        });
        let shape: ResponseShape = serde_json::from_value(json).unwrap();
        match shape {
            ResponseShape::ToolCall(tc) => {
                assert_eq!(tc.calls.len(), 1);
                assert_eq!(tc.calls[0].name, "get_weather");
                assert_eq!(tc.calls[0].arguments, "{}");
            }
            _ => panic!("expected ToolCall variant"),
        }
    }

    #[test]
    fn deserialize_response_shape_tool_call_multiple() {
        let json = serde_json::json!({
            "type": "tool_call",
            "calls": [
                {"name": "search", "arguments": "{}"},
                {"name": "calc", "arguments": "{}"}
            ]
        });
        let shape: ResponseShape = serde_json::from_value(json).unwrap();
        match shape {
            ResponseShape::ToolCall(tc) => {
                assert_eq!(tc.calls.len(), 2);
                assert_eq!(tc.calls[0].name, "search");
                assert_eq!(tc.calls[1].name, "calc");
            }
            _ => panic!("expected ToolCall variant"),
        }
    }

    #[test]
    fn deserialize_response_shape_delay() {
        let json = r#"{"type": "delay"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        assert!(matches!(shape, ResponseShape::Delay));
    }

    #[test]
    fn deserialize_response_shape_unknown_catchall() {
        let json = r#"{"type": "some_future_type"}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        assert!(matches!(shape, ResponseShape::Unknown));
    }

    #[test]
    fn deserialize_http_error_various_status_codes() {
        for (code, msg) in [
            (401, "unauthorized"),
            (429, "rate limited"),
            (500, "server error"),
        ] {
            let json = format!(r#"{{"status": {}, "message": "{}"}}"#, code, msg);
            let err: HttpError = serde_json::from_str(&json).unwrap();
            assert_eq!(err.status, code);
            assert_eq!(err.message, msg);
        }
    }

    #[test]
    fn usage_response_partial_fields() {
        let json = r#"{"type": "usage", "prompt_tokens": 5}"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Usage(u) => {
                assert_eq!(u.prompt_tokens, Some(5));
                assert!(u.completion_tokens.is_none());
            }
            _ => panic!("expected Usage variant"),
        }
    }

    #[test]
    fn scenario_file_empty_scenarios() {
        let json = r#"{"scenarios": []}"#;
        let file: ScenarioFile = serde_json::from_str(json).unwrap();
        assert!(file.scenarios.is_empty());
    }

    #[test]
    fn turn_response_minimal_no_optional_fields() {
        let json = r#"{"response": {"type": "text", "content": "ok"}}"#;
        let turn: TurnResponse = serde_json::from_str(json).unwrap();
        assert!(turn.delay.is_none());
        assert!(turn.error.is_none());
        assert!(turn.stream_interrupt_after.is_none());
    }

    #[test]
    fn deserialize_model_entry() {
        let json = r#"{"id": "gpt-4", "owned_by": "openai"}"#;
        let entry: ModelEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "gpt-4");
        assert_eq!(entry.owned_by, "openai");
    }

    #[test]
    fn deserialize_model_entry_default_owned_by() {
        let json = r#"{"id": "test-model"}"#;
        let entry: ModelEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "test-model");
        assert_eq!(entry.owned_by, "openai");
    }

    #[test]
    fn deserialize_scenario_declaration_with_models() {
        let json = r#"{
            "name": "models-scene",
            "turns": [{"response": {"type": "text", "content": "ok"}}],
            "models": [
                {"id": "gpt-4", "owned_by": "openai"},
                {"id": "claude-3", "owned_by": "anthropic"}
            ]
        }"#;
        let decl: ScenarioDeclaration = serde_json::from_str(json).unwrap();
        assert_eq!(decl.name, "models-scene");
        let models = decl.models.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4");
        assert_eq!(models[1].id, "claude-3");
    }

    #[test]
    fn deserialize_scenario_declaration_without_models() {
        let json = r#"{
            "name": "no-models",
            "turns": [{"response": {"type": "text", "content": "ok"}}]
        }"#;
        let decl: ScenarioDeclaration = serde_json::from_str(json).unwrap();
        assert!(decl.models.is_none());
    }

    #[test]
    fn deserialize_turn_response_legacy_delay_only() {
        // Backward compatibility: old format with only `delay` field
        let json = r#"{"response": {"type": "text", "content": "ok"}, "delay": 100}"#;
        let turn: TurnResponse = serde_json::from_str(json).unwrap();
        assert_eq!(turn.delay, Some(100));
        assert!(turn.first_token_delay.is_none());
        assert!(turn.segment_delay.is_none());
        assert!(turn.stream_interrupt_after.is_none());
    }

    #[test]
    fn deserialize_turn_response_new_format() {
        // New format with all three delay fields + stream_interrupt_after
        let json = r#"{
            "response": {"type": "text", "content": "ok"},
            "first_token_delay": 500,
            "segment_delay": 50,
            "delay": 1000,
            "stream_interrupt_after": 3
        }"#;
        let turn: TurnResponse = serde_json::from_str(json).unwrap();
        assert_eq!(turn.delay, Some(1000));
        assert_eq!(turn.first_token_delay, Some(500));
        assert_eq!(turn.segment_delay, Some(50));
        assert_eq!(turn.stream_interrupt_after, Some(3));
    }

    #[test]
    fn deserialize_turn_response_no_delay_fields() {
        // No delay fields at all
        let json = r#"{"response": {"type": "text", "content": "ok"}}"#;
        let turn: TurnResponse = serde_json::from_str(json).unwrap();
        assert!(turn.delay.is_none());
        assert!(turn.first_token_delay.is_none());
        assert!(turn.segment_delay.is_none());
        assert!(turn.stream_interrupt_after.is_none());
    }

    #[test]
    fn deserialize_turn_response_stream_interrupt_zero() {
        // Boundary: interrupt after 0 events (first event then disconnect)
        let json =
            r#"{"response": {"type": "text", "content": "ok"}, "stream_interrupt_after": 0}"#;
        let turn: TurnResponse = serde_json::from_str(json).unwrap();
        assert_eq!(turn.stream_interrupt_after, Some(0));
    }

    #[test]
    fn deserialize_turn_response_stream_interrupt_absent() {
        // stream_interrupt_after absent defaults to None
        let json = r#"{"response": {"type": "text", "content": "ok"}}"#;
        let turn: TurnResponse = serde_json::from_str(json).unwrap();
        assert!(turn.stream_interrupt_after.is_none());
    }

    // -----------------------------------------------------------------------
    // ReasoningIntensity tests
    // -----------------------------------------------------------------------

    #[test]
    fn reasoning_intensity_default_is_medium() {
        assert_eq!(ReasoningIntensity::default(), ReasoningIntensity::Medium);
    }

    #[test]
    fn reasoning_intensity_serialize_deserialize_low() {
        let json = serde_json::to_string(&ReasoningIntensity::Low).unwrap();
        assert_eq!(json, "\"low\"");
        let parsed: ReasoningIntensity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ReasoningIntensity::Low);
    }

    #[test]
    fn reasoning_intensity_serialize_deserialize_medium() {
        let json = serde_json::to_string(&ReasoningIntensity::Medium).unwrap();
        assert_eq!(json, "\"medium\"");
        let parsed: ReasoningIntensity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ReasoningIntensity::Medium);
    }

    #[test]
    fn reasoning_intensity_serialize_deserialize_high() {
        let json = serde_json::to_string(&ReasoningIntensity::High).unwrap();
        assert_eq!(json, "\"high\"");
        let parsed: ReasoningIntensity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ReasoningIntensity::High);
    }

    #[test]
    fn reasoning_response_default_intensity_is_medium() {
        let resp = ReasoningResponse::default();
        assert_eq!(resp.intensity, ReasoningIntensity::Medium);
    }

    #[test]
    fn reasoning_response_with_intensity_low() {
        let json = r#"{
            "type": "reasoning",
            "content": "42",
            "reasoning": "thinking...",
            "intensity": "low"
        }"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Reasoning(r) => {
                assert_eq!(r.intensity, ReasoningIntensity::Low);
                assert_eq!(r.content, "42");
            }
            _ => panic!("expected Reasoning variant"),
        }
    }

    #[test]
    fn reasoning_response_with_intensity_high() {
        let json = r#"{
            "type": "reasoning",
            "content": "ok",
            "reasoning": "deep thought",
            "intensity": "high"
        }"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Reasoning(r) => {
                assert_eq!(r.intensity, ReasoningIntensity::High);
            }
            _ => panic!("expected Reasoning variant"),
        }
    }

    #[test]
    fn reasoning_response_intensity_defaults_when_absent() {
        let json = r#"{
            "type": "reasoning",
            "content": "text",
            "reasoning": "think"
        }"#;
        let shape: ResponseShape = serde_json::from_str(json).unwrap();
        match shape {
            ResponseShape::Reasoning(r) => {
                assert_eq!(r.intensity, ReasoningIntensity::Medium);
            }
            _ => panic!("expected Reasoning variant"),
        }
    }

    #[test]
    fn reasoning_intensity_not_equal_cross_variant() {
        assert!(ReasoningIntensity::Low != ReasoningIntensity::Medium);
        assert!(ReasoningIntensity::Medium != ReasoningIntensity::High);
        assert!(ReasoningIntensity::Low != ReasoningIntensity::High);
    }
}
