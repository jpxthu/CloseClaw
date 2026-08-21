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
/// Control shapes (Streaming, Error, Delay) carry configuration payloads
/// that the scenario engine extracts into delivery control parameters.
/// Content shapes (Text, Reasoning, ToolCall) produce response blocks.
/// Composite flattens multiple shapes into a single turn.
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

    /// Streaming response — controls delivery segmentation.
    ///
    /// The scenario engine extracts `segment_granularity` and
    /// `segment_delay_ms` into the decision's delivery control
    /// fields (when not overridden by TurnResponse-level values).
    #[serde(rename = "streaming")]
    Streaming(StreamingResponse),

    /// Error response — HTTP status error injection.
    ///
    /// The scenario engine converts this into a `DecisionOutcome::Error`
    /// with the given status, message, and optional retry-after. Fields
    /// mirror `HttpError` for direct conversion.
    #[serde(rename = "error")]
    Error(ErrorResponse),

    /// Delay response — controls timing injection.
    ///
    /// The scenario engine extracts delay parameters into the decision's
    /// delivery control fields (when not overridden by TurnResponse-level
    /// values).
    #[serde(rename = "delay")]
    Delay(DelayResponse),

    /// Token usage report.
    ///
    /// The engine extracts usage data from this shape into the decision.
    /// Usage-only shapes produce no response blocks.
    #[serde(rename = "usage")]
    Usage(UsageResponse),

    /// Composite of multiple shapes in a single turn.
    ///
    /// Enables combinations like "reasoning + tool_call + usage"
    /// in a single response. The protocol layer flattens composite
    /// shapes into individual content blocks.
    #[serde(rename = "composite")]
    Composite(Vec<ResponseShape>),

    /// Catch-all for unimplemented variants (serde default).
    #[serde(other)]
    #[default]
    Unknown,
}

// ---------------------------------------------------------------------------
// Shape payloads
// ---------------------------------------------------------------------------

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

/// Streaming delivery control payload.
///
/// Carries segment granularity and inter-segment delay that the
/// scenario engine extracts into the decision's delivery control
/// fields. When `segment_granularity` is `None` or `0`, the
/// endpoint uses `DEFAULT_SEGMENT_GRANULARITY`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamingResponse {
    /// Number of content units per segment. `None` or `0` means
    /// use the endpoint default (typically 1 token per segment).
    #[serde(default)]
    pub segment_granularity: Option<usize>,
    /// Delay between segments in milliseconds.
    #[serde(default)]
    pub segment_delay_ms: Option<u64>,
    /// Optional token usage report attached to streaming responses.
    #[serde(default)]
    pub usage: Option<UsageResponse>,
}

/// HTTP error response payload.
///
/// Fields mirror `HttpError` so the scenario engine can convert
/// directly into a `DecisionOutcome::Error`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// HTTP status code (e.g. 401, 429, 500).
    pub status: u16,
    /// Error message body.
    pub message: String,
    /// Optional Retry-After header value (seconds).
    #[serde(default)]
    pub retry_after: Option<u64>,
}

/// Delay injection payload.
///
/// Carries first-token, per-segment, and overall delay values
/// that the scenario engine extracts into the decision's delivery
/// control fields. TurnResponse-level values take precedence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DelayResponse {
    /// Overall delay before returning a non-streaming response (ms).
    #[serde(default)]
    pub delay_ms: Option<u64>,
    /// Delay before the first token/segment in streaming mode (ms).
    #[serde(default)]
    pub first_token_delay_ms: Option<u64>,
    /// Delay between segments in streaming mode (ms).
    #[serde(default)]
    pub segment_delay_ms: Option<u64>,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- StreamingResponse tests -------------------------------------------

    #[test]
    fn streaming_full_fields() {
        let json = r#"{"type":"streaming","segment_granularity":5,"segment_delay_ms":100,"usage":{"prompt_tokens":10,"completion_tokens":20}}"#;
        let shape: ResponseShape = serde_json::from_str(json).expect("should deserialize");
        match shape {
            ResponseShape::Streaming(s) => {
                assert_eq!(s.segment_granularity, Some(5));
                assert_eq!(s.segment_delay_ms, Some(100));
                let u = s.usage.unwrap();
                assert_eq!(u.prompt_tokens, Some(10));
                assert_eq!(u.completion_tokens, Some(20));
            }
            _ => panic!("expected Streaming variant"),
        }
    }

    #[test]
    fn streaming_empty_object() {
        let json = r#"{"type":"streaming"}"#;
        let shape: ResponseShape = serde_json::from_str(json).expect("should deserialize");
        match shape {
            ResponseShape::Streaming(s) => {
                assert_eq!(s.segment_granularity, None);
                assert_eq!(s.segment_delay_ms, None);
                assert!(s.usage.is_none());
            }
            _ => panic!("expected Streaming variant"),
        }
    }

    #[test]
    fn streaming_with_unknown_fields() {
        let json = r#"{"type":"streaming","segment_granularity":3,"bogus":true}"#;
        let shape: ResponseShape =
            serde_json::from_str(json).expect("should tolerate unknown fields");
        match shape {
            ResponseShape::Streaming(s) => {
                assert_eq!(s.segment_granularity, Some(3));
            }
            _ => panic!("expected Streaming variant"),
        }
    }

    // -- ErrorResponse tests ------------------------------------------------

    #[test]
    fn error_full_fields() {
        let json = r#"{"type":"error","status":429,"message":"rate limited","retry_after":30}"#;
        let shape: ResponseShape = serde_json::from_str(json).expect("should deserialize");
        match shape {
            ResponseShape::Error(e) => {
                assert_eq!(e.status, 429);
                assert_eq!(e.message, "rate limited");
                assert_eq!(e.retry_after, Some(30));
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn error_without_retry_after() {
        let json = r#"{"type":"error","status":500,"message":"server error"}"#;
        let shape: ResponseShape = serde_json::from_str(json).expect("should deserialize");
        match shape {
            ResponseShape::Error(e) => {
                assert_eq!(e.status, 500);
                assert_eq!(e.message, "server error");
                assert_eq!(e.retry_after, None);
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn error_unknown_fields_tolerated() {
        let json = r#"{"type":"error","status":401,"message":"unauthorized","extra":"ignored"}"#;
        let shape: ResponseShape =
            serde_json::from_str(json).expect("should tolerate unknown fields");
        match shape {
            ResponseShape::Error(e) => {
                assert_eq!(e.status, 401);
                assert_eq!(e.message, "unauthorized");
            }
            _ => panic!("expected Error variant"),
        }
    }

    // -- DelayResponse tests ------------------------------------------------

    #[test]
    fn delay_full_fields() {
        let json =
            r#"{"type":"delay","delay_ms":500,"first_token_delay_ms":200,"segment_delay_ms":50}"#;
        let shape: ResponseShape = serde_json::from_str(json).expect("should deserialize");
        match shape {
            ResponseShape::Delay(d) => {
                assert_eq!(d.delay_ms, Some(500));
                assert_eq!(d.first_token_delay_ms, Some(200));
                assert_eq!(d.segment_delay_ms, Some(50));
            }
            _ => panic!("expected Delay variant"),
        }
    }

    #[test]
    fn delay_empty_object() {
        let json = r#"{"type":"delay"}"#;
        let shape: ResponseShape = serde_json::from_str(json).expect("should deserialize");
        match shape {
            ResponseShape::Delay(d) => {
                assert_eq!(d.delay_ms, None);
                assert_eq!(d.first_token_delay_ms, None);
                assert_eq!(d.segment_delay_ms, None);
            }
            _ => panic!("expected Delay variant"),
        }
    }

    #[test]
    fn delay_partial_fields() {
        let json = r#"{"type":"delay","first_token_delay_ms":100}"#;
        let shape: ResponseShape = serde_json::from_str(json).expect("should deserialize");
        match shape {
            ResponseShape::Delay(d) => {
                assert_eq!(d.delay_ms, None);
                assert_eq!(d.first_token_delay_ms, Some(100));
                assert_eq!(d.segment_delay_ms, None);
            }
            _ => panic!("expected Delay variant"),
        }
    }

    #[test]
    fn delay_unknown_fields_tolerated() {
        let json = r#"{"type":"delay","delay_ms":100,"unknown_key":"value"}"#;
        let shape: ResponseShape =
            serde_json::from_str(json).expect("should tolerate unknown fields");
        match shape {
            ResponseShape::Delay(d) => {
                assert_eq!(d.delay_ms, Some(100));
            }
            _ => panic!("expected Delay variant"),
        }
    }

    // -- Composite with new shapes ------------------------------------------

    #[test]
    fn composite_streaming_and_text() {
        // Build programmatically (Composite serialization via internally
        // tagged representation requires careful format matching; the
        // scenario engine uses ResponseOrComposite, not raw JSON)
        let shape = ResponseShape::Composite(vec![
            ResponseShape::Streaming(StreamingResponse {
                segment_granularity: Some(3),
                ..Default::default()
            }),
            ResponseShape::Text(TextResponse {
                content: "hello".to_string(),
                ..Default::default()
            }),
        ]);
        match shape {
            ResponseShape::Composite(shapes) => {
                assert_eq!(shapes.len(), 2);
                assert!(matches!(shapes[0], ResponseShape::Streaming(_)));
                assert!(matches!(shapes[1], ResponseShape::Text(_)));
            }
            _ => panic!("expected Composite variant"),
        }
    }

    // -- Serialize round-trip ------------------------------------------------

    #[test]
    fn streaming_roundtrip() {
        let original = StreamingResponse {
            segment_granularity: Some(10),
            segment_delay_ms: Some(25),
            usage: None,
        };
        let shape = ResponseShape::Streaming(original.clone());
        let json = serde_json::to_string(&shape).unwrap();
        let deserialized: ResponseShape = serde_json::from_str(&json).unwrap();
        match deserialized {
            ResponseShape::Streaming(s) => {
                assert_eq!(s.segment_granularity, original.segment_granularity);
                assert_eq!(s.segment_delay_ms, original.segment_delay_ms);
            }
            _ => panic!("expected Streaming"),
        }
    }

    #[test]
    fn error_roundtrip() {
        let original = ErrorResponse {
            status: 429,
            message: "too many requests".to_string(),
            retry_after: None,
        };
        let shape = ResponseShape::Error(original.clone());
        let json = serde_json::to_string(&shape).unwrap();
        let deserialized: ResponseShape = serde_json::from_str(&json).unwrap();
        match deserialized {
            ResponseShape::Error(e) => {
                assert_eq!(e.status, original.status);
                assert_eq!(e.message, original.message);
                assert_eq!(e.retry_after, original.retry_after);
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn delay_roundtrip() {
        let original = DelayResponse {
            delay_ms: Some(300),
            first_token_delay_ms: None,
            segment_delay_ms: Some(10),
        };
        let shape = ResponseShape::Delay(original.clone());
        let json = serde_json::to_string(&shape).unwrap();
        let deserialized: ResponseShape = serde_json::from_str(&json).unwrap();
        match deserialized {
            ResponseShape::Delay(d) => {
                assert_eq!(d.delay_ms, original.delay_ms);
                assert_eq!(d.first_token_delay_ms, original.first_token_delay_ms);
                assert_eq!(d.segment_delay_ms, original.segment_delay_ms);
            }
            _ => panic!("expected Delay"),
        }
    }
}
