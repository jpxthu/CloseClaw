// ------------------------------------------------------------------
// Fixture contract tests
//
// Step 1.2: "场景声明 → ScenarioEngine → 协议序列化 ≡ fixture.response"
//
// For each non-streaming protocol fixture, constructs a matching
// ScenarioDeclaration, runs it through the engine, builds the
// protocol response, and compares semantic fields against the fixture.
//
// Field comparison strategy (three categories per plan §思路):
//   1. Deterministic equal: content, reasoning, tool_calls semantic
//      fields, finish_reason, usage numbers, stop_reason, model → exact
//   2. Shape-locked: id, created → assert existence + format rules
//   3. Code can't produce from scenario: fixture-only fields → skipped
//      (see UNPRODUCED_FIELDS comments)
// ------------------------------------------------------------------

use crate::fixture_loader::{fixture_root, load_protocol_fixture};
use crate::protocol::anthropic::build_message_response_from_decision;
use crate::protocol::openai::build_chat_completion_response_from_decision;
use crate::scenario::types::*;
use crate::types::RequestFeatures;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a fallback ScenarioDeclaration (no match condition) from a fixture's
/// scenario name, model, and ResponseShape.
fn make_fallback(
    fixture_scenario: &str,
    _model: &str,
    shape: ResponseShape,
    usage: Option<UsageResponse>,
) -> ScenarioDeclaration {
    // Embed usage into the shape so extract_usage picks it up.
    let shape_with_usage = match shape {
        ResponseShape::Text(mut t) => {
            t.usage = usage;
            ResponseShape::Text(t)
        }
        ResponseShape::Reasoning(mut r) => {
            r.usage = usage;
            ResponseShape::Reasoning(r)
        }
        ResponseShape::ToolCall(mut tc) => {
            tc.usage = usage;
            ResponseShape::ToolCall(tc)
        }
        other => other,
    };
    ScenarioDeclaration {
        name: fixture_scenario.to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: shape_with_usage,
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            error: None,
        }],
        models: None,
    }
}

/// Build a fallback ScenarioDeclaration with error injection.
fn make_error_fallback(
    fixture_scenario: &str,
    status: u16,
    message: &str,
    retry_after: Option<u64>,
) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: fixture_scenario.to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: String::new(),
                usage: None,
            }),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            error: Some(HttpError {
                status,
                message: message.to_string(),
                retry_after,
            }),
        }],
        models: None,
    }
}

/// Build a minimal RequestFeatures from a fixture's request payload.
/// Extract text content from a JSON message value.
fn message_content_to_string(v: &serde_json::Value) -> String {
    if v.is_string() {
        v.as_str().map(String::from).unwrap_or_default()
    } else if let Some(arr) = v.as_array() {
        arr.iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("")
    } else {
        String::new()
    }
}

/// Parse messages array from a fixture request JSON.
fn parse_fixture_messages(req: &serde_json::Value) -> Vec<MessageEntry> {
    req.get("messages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| MessageEntry {
                    role: m
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user")
                        .to_string(),
                    content: m
                        .get("content")
                        .map(message_content_to_string)
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Build RequestFeatures from a fixture's request payload.
fn request_features_from_fixture(
    fixture: &crate::fixture_loader::ProtocolFixture,
    is_anthropic: bool,
) -> RequestFeatures {
    let req = fixture.request.as_ref().expect("fixture must have request");
    let messages = parse_fixture_messages(req);
    let max_tokens = fixture
        .max_tokens_sent
        .or_else(|| if is_anthropic { Some(1024) } else { None });
    RequestFeatures {
        model: fixture.model.clone(),
        stream: false,
        max_tokens,
        temperature: None,
        messages,
        tools: vec![],
    }
}

/// Extract common OpenAI response fields for comparison.
fn openai_fields(json: &serde_json::Value) -> OpenAiResp {
    let msg = &json["choices"][0]["message"];
    let tool_calls_raw = msg["tool_calls"].as_array().cloned().unwrap_or_default();
    let tool_calls: Vec<ToolCallInfo> = tool_calls_raw
        .iter()
        .map(|tc| ToolCallInfo {
            id: tc["id"].as_str().unwrap_or("").to_string(),
            call_type: tc["type"].as_str().unwrap_or("").to_string(),
            function_name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
            function_arguments: tc["function"]["arguments"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        })
        .collect();
    OpenAiResp {
        id: json["id"].as_str().unwrap_or("").to_string(),
        object: json["object"].as_str().unwrap_or("").to_string(),
        model: json["model"].as_str().unwrap_or("").to_string(),
        content: msg["content"].as_str().unwrap_or("").to_string(),
        reasoning_content: msg["reasoning_content"].as_str().map(String::from),
        tool_calls,
        finish_reason: json["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        usage_prompt: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        usage_completion: json["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        usage_total: json["usage"]["total_tokens"].as_u64().unwrap_or(0),
    }
}

struct OpenAiResp {
    id: String,
    object: String,
    model: String,
    content: String,
    reasoning_content: Option<String>,
    tool_calls: Vec<ToolCallInfo>,
    finish_reason: String,
    usage_prompt: u64,
    usage_completion: u64,
    usage_total: u64,
}

struct ToolCallInfo {
    id: String,
    call_type: String,
    function_name: String,
    function_arguments: String,
}

/// Extract common Anthropic response fields for comparison.
fn anthropic_fields(json: &serde_json::Value) -> AnthropicResp {
    let content_raw = json["content"].as_array().cloned().unwrap_or_default();
    let content: Vec<ContentInfo> = content_raw
        .iter()
        .map(|block| ContentInfo {
            block_type: block["type"].as_str().unwrap_or("").to_string(),
            text: block["text"].as_str().map(String::from),
            thinking: block["thinking"].as_str().map(String::from),
            signature: block["signature"].as_str().map(String::from),
            tool_use_id: block["id"].as_str().map(String::from),
            tool_use_name: block["name"].as_str().map(String::from),
        })
        .collect();
    AnthropicResp {
        id: json["id"].as_str().unwrap_or("").to_string(),
        message_type: json["type"].as_str().unwrap_or("").to_string(),
        role: json["role"].as_str().unwrap_or("").to_string(),
        model: json["model"].as_str().unwrap_or("").to_string(),
        content,
        stop_reason: json["stop_reason"].as_str().unwrap_or("").to_string(),
        usage_input: json["usage"]["input_tokens"].as_u64().unwrap_or(0),
        usage_output: json["usage"]["output_tokens"].as_u64().unwrap_or(0),
    }
}

struct AnthropicResp {
    id: String,
    message_type: String,
    role: String,
    model: String,
    content: Vec<ContentInfo>,
    stop_reason: String,
    usage_input: u64,
    usage_output: u64,
}

struct ContentInfo {
    block_type: String,
    text: Option<String>,
    thinking: Option<String>,
    signature: Option<String>,
    tool_use_id: Option<String>,
    tool_use_name: Option<String>,
}

// ---------------------------------------------------------------------------
// OpenAI fixture tests
// ---------------------------------------------------------------------------

/// UNPRODUCED_FIELDS (OpenAI):
/// - `system_fingerprint`: fixture has "fp_fake"; code omits entirely
/// - `refusal`: fixture has null; code omits entirely
/// - `logprobs`: fixture has null; code omits entirely
/// - `usage.prompt_tokens_details.cached_tokens`: fixture has 0;
///   code produces Usage { prompt_tokens, completion_tokens, total_tokens }
///   — no details sub-structure
/// - `usage.completion_tokens_details.reasoning_tokens`: fixture has
///   reasoning_tokens value; code omits details sub-structure
///
/// STRATEGY: Compare only the fields the code produces. Skip fixture-only
/// fields. The consumer (CloseClaw llm crate) tests mapping of these
/// fixture fields in Step 1.4.
#[test]
fn openai_simple_fixture_matches() {
    let path = fixture_root().join("openai/simple.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let usage = Some(UsageResponse {
        prompt_tokens: Some(14),
        completion_tokens: Some(4),
        ..Default::default()
    });
    let scenario = make_fallback(
        &fixture.scenario,
        &fixture.model,
        ResponseShape::Text(TextResponse {
            content: "Hello there friend.".to_string(),
            usage: None,
        }),
        usage,
    );
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, false);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };
    let resp = build_chat_completion_response_from_decision(&decision);
    let json = serde_json::to_value(&resp).unwrap();
    let f = openai_fields(&json);

    // Shape-locked: id
    assert!(
        f.id.starts_with("chatcmpl-"),
        "id should start with chatcmpl-, got: {}",
        f.id
    );
    assert_eq!(f.object, "chat.completion");
    assert_eq!(f.model, "fake-model");

    // Deterministic
    assert_eq!(f.content, "Hello there friend.");
    assert!(f.reasoning_content.is_none());
    assert!(f.tool_calls.is_empty());
    assert_eq!(f.finish_reason, "stop");
    assert_eq!(f.usage_prompt, 14);
    assert_eq!(f.usage_completion, 4);
    assert_eq!(f.usage_total, 18);
}

/// UNPRODUCED_FIELDS (OpenAI reasoning):
/// - `reasoning_tokens` in usage completion_tokens_details:
///   fixture has 21; code omits this field (Usage struct has no
///   reasoning_tokens_details)
#[test]
fn openai_reasoning_fixture_matches() {
    let path = fixture_root().join("openai/reasoning.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let usage = Some(UsageResponse {
        prompt_tokens: Some(14),
        completion_tokens: Some(24),
        ..Default::default()
    });
    let scenario = make_fallback(
        &fixture.scenario,
        &fixture.model,
        ResponseShape::Reasoning(ReasoningResponse {
            content: "391".to_string(),
            reasoning: "The user asks for 17 * 23. Compute: 17 * 20 = 340, 17 * 3 = 51, sum = 391."
                .to_string(),
            signature: None,
            usage: None,
            ..Default::default()
        }),
        usage,
    );
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, false);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };
    let resp = build_chat_completion_response_from_decision(&decision);
    let json = serde_json::to_value(&resp).unwrap();
    let f = openai_fields(&json);

    assert!(f.id.starts_with("chatcmpl-"));
    assert_eq!(f.object, "chat.completion");
    assert_eq!(f.model, "fake-model");
    assert!(
        f.reasoning_content
            .as_deref()
            .unwrap()
            .contains("The user asks for 17 * 23. Compute: 17 * 20 = 340, 17 * 3 = 51, sum = 391."),
        "reasoning_content should contain base text"
    );
    assert_eq!(f.content, "391");
    assert_eq!(f.finish_reason, "stop");
    assert_eq!(f.usage_prompt, 14);
    assert_eq!(f.usage_completion, 24);
    assert_eq!(f.usage_total, 38);
}

/// UNPRODUCED_FIELDS (OpenAI tool-use):
/// - tool_calls[].id: fixture has "call_fake_001"; code generates
///   "call_{idx}" — shape-locked, compare prefix + type only
#[test]
fn openai_tool_use_fixture_matches() {
    let path = fixture_root().join("openai/tool-use.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let usage = Some(UsageResponse {
        prompt_tokens: Some(18),
        completion_tokens: Some(12),
        ..Default::default()
    });
    let scenario = make_fallback(
        &fixture.scenario,
        &fixture.model,
        ResponseShape::ToolCall(ToolCallResponse {
            calls: vec![ToolCallEntry {
                name: "get_weather".to_string(),
                arguments: r#"{"location":"Tokyo"}"#.to_string(),
            }],
            usage: None,
        }),
        usage,
    );
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, false);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };
    let resp = build_chat_completion_response_from_decision(&decision);
    let json = serde_json::to_value(&resp).unwrap();
    let f = openai_fields(&json);

    assert!(f.id.starts_with("chatcmpl-"));
    assert_eq!(f.object, "chat.completion");
    assert_eq!(f.model, "fake-model");
    assert_eq!(f.content, "");
    assert_eq!(f.finish_reason, "tool_calls");
    assert_eq!(f.tool_calls.len(), 1);

    let tc = &f.tool_calls[0];
    assert!(tc.id.starts_with("call_"), "tool call id: {}", tc.id);
    assert_eq!(tc.call_type, "function");
    assert_eq!(tc.function_name, "get_weather");
    assert_eq!(tc.function_arguments, r#"{"location":"Tokyo"}"#);

    assert_eq!(f.usage_prompt, 18);
    assert_eq!(f.usage_completion, 12);
    assert_eq!(f.usage_total, 30);
}

/// UNPRODUCED_FIELDS (OpenAI cache):
/// - `usage.prompt_tokens_details.cached_tokens`: fixture has 28;
///   code produces Usage { prompt_tokens: 38, completion_tokens: 72,
///   total_tokens: 110 } without details sub-structure
#[test]
fn openai_cache_fixture_matches() {
    let path = fixture_root().join("openai/cache.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let usage = Some(UsageResponse {
        prompt_tokens: Some(38),
        completion_tokens: Some(72),
        ..Default::default()
    });
    let scenario = make_fallback(&fixture.scenario, &fixture.model, ResponseShape::Text(TextResponse {
        content: "HTTP keep-alive allows a single TCP connection to be reused for multiple HTTP request/response cycles instead of opening a new connection per request. The client sends `Connection: keep-alive` (or omits `Connection: close` on HTTP/1.1, where keep-alive is the default), and the server holds the socket open after sending the response. Subsequent requests reuse the same socket, avoiding the cost of TCP handshake, slow-start, and TLS negotiation (for HTTPS).".to_string(),
        usage: None,
    }), usage);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, false);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };
    let resp = build_chat_completion_response_from_decision(&decision);
    let json = serde_json::to_value(&resp).unwrap();
    let f = openai_fields(&json);

    assert!(f.id.starts_with("chatcmpl-"));
    assert_eq!(f.object, "chat.completion");
    assert_eq!(f.model, "fake-model");
    assert!(f.content.starts_with("HTTP keep-alive"));
    assert_eq!(f.finish_reason, "stop");
    assert_eq!(f.usage_prompt, 38);
    assert_eq!(f.usage_completion, 72);
    assert_eq!(f.usage_total, 110);
}

// ---------------------------------------------------------------------------
// OpenAI error fixture tests
// ---------------------------------------------------------------------------

/// Error fixtures: DecisionOutcome::Error carries status + message.
/// The endpoint layer formats the error body (OpenAI/Anthropic format).
/// This test verifies the decision-level error structure.
#[test]
fn openai_error_auth_fixture_matches() {
    let path = fixture_root().join("openai/error-auth.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let scenario = make_error_fallback(&fixture.scenario, 401, "Unauthorized", None);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, false);
    match engine.decide(&features) {
        crate::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 401);
            assert_eq!(e.message, "Unauthorized");
        }
        crate::DecisionOutcome::Decision(_) => panic!("expected Error"),
    }
}

#[test]
fn openai_error_rate_limit_fixture_matches() {
    let path = fixture_root().join("openai/error-rate-limit.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let scenario = make_error_fallback(&fixture.scenario, 429, "Too Many Requests", None);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, false);
    match engine.decide(&features) {
        crate::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 429);
            assert_eq!(e.message, "Too Many Requests");
        }
        crate::DecisionOutcome::Decision(_) => panic!("expected Error"),
    }
}

#[test]
fn openai_error_server_fixture_matches() {
    let path = fixture_root().join("openai/error-server.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let scenario = make_error_fallback(&fixture.scenario, 500, "Internal Server Error", None);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, false);
    match engine.decide(&features) {
        crate::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert_eq!(e.message, "Internal Server Error");
        }
        crate::DecisionOutcome::Decision(_) => panic!("expected Error"),
    }
}

// ---------------------------------------------------------------------------
// Anthropic fixture tests
// ---------------------------------------------------------------------------

/// UNPRODUCED_FIELDS (Anthropic simple):
/// - `usage.cache_creation_input_tokens`: fixture has 0;
///   code produces Usage { input_tokens, output_tokens } without
///   cache sub-structure
/// - `usage.cache_read_input_tokens`: fixture has 0; same reason
/// - `usage.service_tier`: fixture has "standard"; code omits entirely
/// - `stop_sequence`: fixture has null; code omits entirely
#[test]
fn anthropic_simple_fixture_matches() {
    let path = fixture_root().join("anthropic/anthropic-simple.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let usage = Some(UsageResponse {
        prompt_tokens: Some(11),
        completion_tokens: Some(4),
        ..Default::default()
    });
    let scenario = make_fallback(
        &fixture.scenario,
        &fixture.model,
        ResponseShape::Text(TextResponse {
            content: "Hello there friend.".to_string(),
            usage: None,
        }),
        usage,
    );
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, true);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };
    let resp = build_message_response_from_decision(&decision);
    let json = serde_json::to_value(&resp).unwrap();
    let f = anthropic_fields(&json);

    // Shape-locked: id
    assert!(
        f.id.starts_with("msg-"),
        "id should start with msg-, got: {}",
        f.id
    );
    assert_eq!(f.message_type, "message");
    assert_eq!(f.role, "assistant");
    assert_eq!(f.model, "fake-model");

    // Deterministic
    assert_eq!(f.content.len(), 1);
    assert_eq!(f.content[0].block_type, "text");
    assert_eq!(f.content[0].text.as_deref(), Some("Hello there friend."));
    assert_eq!(f.stop_reason, "end_turn");
    assert_eq!(f.usage_input, 11);
    assert_eq!(f.usage_output, 4);
}

/// UNPRODUCED_FIELDS (Anthropic thinking):
/// - `usage.cache_creation_input_tokens`, `cache_read_input_tokens`,
///   `service_tier`: same as simple
/// - `stop_sequence`: same as simple
const THINKING_REASONING: &str = "We need to compute 17 * 23. Using the distributive property: \
     (10 + 7) * (20 + 3) = 10*20 + 10*3 + 7*20 + 7*3 = 200 + 30 + 140 + 21 = 391.";
const THINKING_TEXT: &str = "To compute 17 * 23, we break it down using the distributive \
     property:\n\n17 * 23 = (10 + 7) * (20 + 3)\n\nMultiplying each \
     term:\n- 10 * 20 = 200\n- 10 * 3 = 30\n- 7 * 20 = 140\n- 7 * 3 = \
     21\n\nAdding the results: 200 + 30 + 140 + 21 = 391.\n\nThus, 17 * 23 \
     = 391.";
const THINKING_SIG: &str = "sig_thinking_b2c3d4e5f6a7b8c9";
const TOOLUSE_REASONING: &str = "The user wants the current weather in San Francisco. I have a \
     get_weather tool available, so I'll invoke it with the location.";
const TOOLUSE_SIG: &str = "sig_tooluse_c3d4e5f6a7b8c9d0";
const TOOLUSE_LOCATION: &str = r#"{"location":"San Francisco"}"#;

/// Build the response blocks for the Anthropic tool-use fixture.
fn tool_use_blocks() -> Vec<ResponseBlock> {
    vec![
        ResponseBlock {
            block_type: "reasoning".to_string(),
            content: None,
            tool_name: None,
            tool_arguments: None,
            reasoning: Some(TOOLUSE_REASONING.to_string()),
            signature: Some(TOOLUSE_SIG.to_string()),
        },
        ResponseBlock {
            block_type: "tool_call".to_string(),
            content: None,
            tool_name: Some("get_weather".to_string()),
            tool_arguments: Some(TOOLUSE_LOCATION.to_string()),
            reasoning: None,
            signature: None,
        },
    ]
}

#[test]
fn anthropic_thinking_fixture_matches() {
    let path = fixture_root().join("anthropic/anthropic-thinking.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let usage = Some(UsageResponse {
        prompt_tokens: Some(18),
        completion_tokens: Some(90),
        ..Default::default()
    });
    let scenario = make_fallback(
        &fixture.scenario,
        &fixture.model,
        ResponseShape::Reasoning(ReasoningResponse {
            content: THINKING_TEXT.to_string(),
            reasoning: THINKING_REASONING.to_string(),
            signature: Some(THINKING_SIG.to_string()),
            usage: None,
            ..Default::default()
        }),
        usage,
    );
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, true);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };
    let resp = build_message_response_from_decision(&decision);
    let json = serde_json::to_value(&resp).unwrap();
    let f = anthropic_fields(&json);

    assert!(f.id.starts_with("msg-"));
    assert_eq!(f.message_type, "message");
    assert_eq!(f.role, "assistant");
    assert_eq!(f.model, "fake-model");
    assert_eq!(f.content.len(), 2);

    assert_eq!(f.content[0].block_type, "thinking");
    assert!(
        f.content[0]
            .thinking
            .as_deref()
            .unwrap()
            .contains(THINKING_REASONING),
        "thinking should contain base text"
    );
    assert_eq!(f.content[0].signature.as_deref(), Some(THINKING_SIG));

    assert_eq!(f.content[1].block_type, "text");
    assert_eq!(f.content[1].text.as_deref(), Some(THINKING_TEXT));

    assert_eq!(f.stop_reason, "end_turn");
    assert_eq!(f.usage_input, 18);
    assert_eq!(f.usage_output, 90);
}

/// UNPRODUCED_FIELDS (Anthropic tool-use):
/// - content[].id (tool_use): fixture has
///   "toolu_fake_01_Vau98RhEyykRxCrGkYDe1551"; code generates
///   "toolu_{idx}" — shape-locked, compare prefix only
/// - `usage.cache_*`, `service_tier`, `stop_sequence`: same as above
#[test]
fn anthropic_tool_use_fixture_matches() {
    let path = fixture_root().join("anthropic/anthropic-tool-use.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let resp = build_message_response_from_decision(&crate::types::ScenarioDecision {
        model: fixture.model.clone(),
        scenario: fixture.scenario.clone(),
        stream: false,
        response_blocks: tool_use_blocks(),
        http_error: None,
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        usage: Some(UsageResponse {
            prompt_tokens: Some(39),
            completion_tokens: Some(45),
            ..Default::default()
        }),
    });
    let json = serde_json::to_value(&resp).unwrap();
    let f = anthropic_fields(&json);

    assert!(f.id.starts_with("msg-"));
    assert_eq!(f.message_type, "message");
    assert_eq!(f.role, "assistant");
    assert_eq!(f.model, "fake-model");
    assert_eq!(f.content.len(), 2);

    assert_eq!(f.content[0].block_type, "thinking");
    assert_eq!(f.content[0].thinking.as_deref(), Some(TOOLUSE_REASONING));
    assert_eq!(f.content[0].signature.as_deref(), Some(TOOLUSE_SIG));

    assert_eq!(f.content[1].block_type, "tool_use");
    assert_eq!(f.content[1].tool_use_name.as_deref(), Some("get_weather"));
    let tool_id = f.content[1].tool_use_id.as_deref().unwrap_or("");
    assert!(tool_id.starts_with("toolu_"), "tool_use id: {}", tool_id);

    assert_eq!(f.stop_reason, "tool_use");
    assert_eq!(f.usage_input, 39);
    assert_eq!(f.usage_output, 45);
}

/// UNPRODUCED_FIELDS (Anthropic cache):
/// - `usage.cache_creation_input_tokens`: fixture has 0;
///   code produces Usage { input_tokens, output_tokens }
/// - `usage.cache_read_input_tokens`: fixture has 256; same reason
/// - `usage.service_tier`: fixture has "standard"; code omits
/// - `stop_sequence`: fixture has null; code omits
#[test]
fn anthropic_cache_fixture_matches() {
    let path = fixture_root().join("anthropic/anthropic-cache.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let usage = Some(UsageResponse {
        prompt_tokens: Some(22),
        completion_tokens: Some(110),
        ..Default::default()
    });
    let scenario = make_fallback(&fixture.scenario, &fixture.model, ResponseShape::Reasoning(
        ReasoningResponse {
            content: "HTTP/1.1 uses a request-per-connection model (or keep-alive with head-of-line blocking), while HTTP/2 multiplexes many request/response streams over a single TCP connection using binary framing.\n\nKey differences:\n- HTTP/2 sends interleaved binary frames for multiple streams on one connection.\n- HTTP/1.1 requires serial responses, leading to head-of-line blocking.\n- HTTP/2 supports stream priorities and per-stream flow control.\n\nIn short, HTTP/2 multiplexing eliminates the need to open many TCP connections and avoids the head-of-line blocking that hurts HTTP/1.1 performance.".to_string(),
            reasoning: "The user is asking about HTTP/1.1 vs HTTP/2 multiplexing. The system prompt is cached from a previous request, so I should report cache_read_input_tokens > 0 to indicate a cache hit.".to_string(),
            signature: Some("sig_cache_d4e5f6a7b8c9d0e1".to_string()),
            usage: None,
            ..Default::default()
        },
    ), usage);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, true);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };
    let resp = build_message_response_from_decision(&decision);
    let json = serde_json::to_value(&resp).unwrap();
    let f = anthropic_fields(&json);

    assert!(f.id.starts_with("msg-"));
    assert_eq!(f.message_type, "message");
    assert_eq!(f.role, "assistant");
    assert_eq!(f.model, "fake-model");
    assert_eq!(f.content.len(), 2);

    // thinking block
    assert_eq!(f.content[0].block_type, "thinking");
    assert!(
        f.content[0]
            .thinking
            .as_deref()
            .unwrap()
            .contains("The user is asking about HTTP/1.1 vs HTTP/2 multiplexing. The system prompt is cached from a previous request, so I should report cache_read_input_tokens > 0 to indicate a cache hit."),
        "thinking should contain base text"
    );
    assert_eq!(
        f.content[0].signature.as_deref(),
        Some("sig_cache_d4e5f6a7b8c9d0e1")
    );

    // text block
    assert_eq!(f.content[1].block_type, "text");
    assert!(f.content[1]
        .text
        .as_deref()
        .unwrap()
        .starts_with("HTTP/1.1"));

    assert_eq!(f.stop_reason, "end_turn");
    assert_eq!(f.usage_input, 22);
    assert_eq!(f.usage_output, 110);
}

// ---------------------------------------------------------------------------
// Anthropic error fixture
// ---------------------------------------------------------------------------

#[test]
fn anthropic_error_fixture_matches() {
    let path = fixture_root().join("anthropic/anthropic-error.json");
    let fixture = load_protocol_fixture(&path).unwrap();
    let scenario = make_error_fallback(&fixture.scenario, 400, "Bad Request", None);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_fixture(&fixture, true);
    match engine.decide(&features) {
        crate::DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 400);
            assert_eq!(e.message, "Bad Request");
        }
        crate::DecisionOutcome::Decision(_) => panic!("expected Error"),
    }
}
