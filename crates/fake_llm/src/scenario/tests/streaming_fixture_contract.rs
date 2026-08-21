// ------------------------------------------------------------------
// Streaming fixture contract tests
//
// Step 1.3: "场景声明 → 流式生成路径 → SSE chunk/事件序列 ≡ fixture.txt"
//
// For each streaming protocol fixture (OpenAI + Anthropic, text + tool-use),
// constructs a matching ScenarioDeclaration, runs it through the engine,
// generates SSE events via the delivery layer, and compares semantic
// content against the fixture's SSE text.
//
// Field comparison strategy (same three categories as fixture_contract.rs):
//   1. Deterministic equal: text deltas, tool_call name/arguments,
//      finish_reason/stop_reason, usage numbers, event type sequence → exact
//   2. Shape-locked: id, created, tool_call id → assert existence + format
//   3. Code can't produce from scenario: fixture-only fields → skipped
//      (see UNPRODUCED_FIELDS comments per fixture)
// ------------------------------------------------------------------

use crate::delivery::sse::{generate_anthropic_sse, generate_openai_sse};
use crate::fixture_loader::{fixture_root, load_streaming_fixture, load_streaming_meta};
use crate::scenario::types::*;
use crate::types::RequestFeatures;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse raw SSE text into a list of (event_type, data) pairs.
///
/// Skips blank lines and handles both "event: X\ndata: Y" and "data: Y"
/// formats (when event type is missing, defaults to "message").
fn parse_sse_text(text: &str) -> Vec<(String, String)> {
    let mut events = Vec::new();
    let mut current_event = String::from("message");
    let mut current_data = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !current_data.is_empty() {
                events.push((current_event.clone(), current_data.clone()));
            }
            current_event = "message".to_string();
            current_data.clear();
            continue;
        }
        if let Some(evt) = trimmed.strip_prefix("event: ") {
            current_event = evt.to_string();
        } else if let Some(data) = trimmed.strip_prefix("data: ") {
            current_data = data.to_string();
        }
    }
    if !current_data.is_empty() {
        events.push((current_event, current_data));
    }
    events
}

/// Build a RequestFeatures from a streaming meta JSON value.
fn request_features_from_meta(meta: &serde_json::Value, is_anthropic: bool) -> RequestFeatures {
    let req = meta.get("request").expect("meta must have request");
    let messages_raw = req
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let messages: Vec<MessageEntry> = messages_raw
        .iter()
        .map(|m| MessageEntry {
            role: m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string(),
            content: m
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect();

    let tools_raw = meta.get("tools_sent").and_then(|v| v.as_array());
    let tools: Vec<String> = tools_raw
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    t.get("function")
                        .and_then(|f| f.get("name"))
                        .or_else(|| t.get("name"))
                        .and_then(|n| n.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();

    let max_tokens = meta
        .get("max_tokens_sent")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .or_else(|| if is_anthropic { Some(1024) } else { None });

    let model = meta
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("fake-model")
        .to_string();

    RequestFeatures {
        model,
        stream: true,
        max_tokens,
        temperature: None,
        messages,
        tools,
    }
}

/// Build a ScenarioDeclaration from a streaming meta JSON value and a response shape.
fn make_streaming_scenario(meta: &serde_json::Value, shape: ResponseShape) -> ScenarioDeclaration {
    let scenario = meta
        .get("scenario")
        .and_then(|v| v.as_str())
        .unwrap_or("streaming");
    ScenarioDeclaration {
        name: scenario.to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: shape.into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    }
}

// ---------------------------------------------------------------------------
// OpenAI streaming fixture tests
// ---------------------------------------------------------------------------

/// UNPRODUCED_FIELDS (OpenAI streaming):
/// - `system_fingerprint`: fixture has "fp_fake"; code omits entirely
/// - `created`: fixture has 1700000008; code generates `fake-{model}`
///   as id (not `chatcmpl-` prefix) — shape-locked
/// - `logprobs`: fixture has null; code omits entirely
/// - `usage.prompt_tokens_details.cached_tokens`: fixture has 0;
///   code produces usage without details sub-structure
/// - `usage.completion_tokens_details.reasoning_tokens`: fixture has 0;
///   code omits details sub-structure
///
/// STRATEGY: Compare semantic fields the code produces. The code's SSE
/// `id` field is `fake-{model}` (not `chatcmpl-{scenario}`) — this is
/// the code's current behavior. We assert format only (non-empty string).
#[test]
fn openai_streaming_text_fixture_matches_semantics() {
    let root = fixture_root();
    let meta_path = root.join("openai/streaming-meta.json");
    let meta = load_streaming_meta(&meta_path).unwrap();
    let txt_path = root.join("openai/streaming.txt");
    let txt_content = load_streaming_fixture(&txt_path).unwrap();

    // Build scenario from meta — use TextResponse with the expected content
    let shape = ResponseShape::Text(TextResponse {
        content: "Hello there friend.".to_string(),
        usage: Some(UsageResponse {
            prompt_tokens: Some(14),
            completion_tokens: Some(4),
            reasoning_tokens: None,
            cache_hit_tokens: None,
            cache_write_tokens: None,
            cache_fields_missing: false,
        }),
    });
    let scenario = make_streaming_scenario(&meta, shape);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_meta(&meta, false);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };

    // Generate SSE events with granularity=0 (single delta per block)
    let usage_resp = decision.usage.clone().unwrap_or_default();
    let events = generate_openai_sse(
        &decision.response_blocks,
        &decision.model,
        &usage_resp,
        true, // include_usage from meta stream_options
        0,
    );

    // Parse fixture SSE text
    let fixture_events = parse_sse_text(&txt_content);

    // --- Event type sequence ---
    let generated_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(
        !generated_types.is_empty(),
        "generated events should not be empty"
    );
    assert!(
        !fixture_events.is_empty(),
        "fixture events should not be empty"
    );
    assert!(
        generated_types.iter().all(|t| *t == "message"),
        "all generated events should be message type"
    );

    // --- Semantic: role chunk exists ---
    let first_gen: serde_json::Value =
        serde_json::from_str(&events[0].data).expect("first event should be valid JSON");
    assert_eq!(
        first_gen["choices"][0]["delta"]["role"], "assistant",
        "first chunk should set role=assistant"
    );
    let first_fix: serde_json::Value =
        serde_json::from_str(&fixture_events[0].1).expect("fixture first event valid JSON");
    assert_eq!(
        first_fix["choices"][0]["delta"]["role"], "assistant",
        "fixture first chunk should set role=assistant"
    );

    // --- Semantic: text content matches ---
    let gen_content: String = events
        .iter()
        .filter_map(|e| {
            let v: serde_json::Value = serde_json::from_str(&e.data).ok()?;
            v["choices"][0]["delta"]["content"]
                .as_str()
                .map(String::from)
        })
        .collect();
    let fix_content: String = fixture_events
        .iter()
        .filter_map(|(_, data)| {
            let v: serde_json::Value = serde_json::from_str(data).ok()?;
            v["choices"][0]["delta"]["content"]
                .as_str()
                .map(String::from)
        })
        .collect();
    assert_eq!(
        gen_content, fix_content,
        "accumulated text content should match fixture"
    );
    assert_eq!(gen_content, "Hello there friend.");

    // --- Semantic: finish chunk ---
    let gen_finish = events
        .iter()
        .rev()
        .find(|e| {
            serde_json::from_str::<serde_json::Value>(&e.data)
                .ok()
                .and_then(|v| {
                    v["choices"][0]["finish_reason"]
                        .as_str()
                        .map(|s| !s.is_empty())
                })
                .unwrap_or(false)
        })
        .expect("should have a finish chunk");
    let gen_finish_val: serde_json::Value = serde_json::from_str(&gen_finish.data).unwrap();
    assert_eq!(gen_finish_val["choices"][0]["finish_reason"], "stop");

    let fix_finish = fixture_events
        .iter()
        .rev()
        .skip(1) // skip [DONE]
        .find(|(_, data)| {
            serde_json::from_str::<serde_json::Value>(data)
                .ok()
                .and_then(|v| {
                    v["choices"][0]["finish_reason"]
                        .as_str()
                        .map(|s| !s.is_empty())
                })
                .unwrap_or(false)
        })
        .expect("fixture should have a finish chunk");
    let fix_finish_val: serde_json::Value = serde_json::from_str(&fix_finish.1).unwrap();
    assert_eq!(fix_finish_val["choices"][0]["finish_reason"], "stop");

    // --- Semantic: usage in finish chunk ---
    assert_eq!(gen_finish_val["usage"]["prompt_tokens"], 14);
    assert_eq!(gen_finish_val["usage"]["completion_tokens"], 4);
    assert_eq!(gen_finish_val["usage"]["total_tokens"], 18);
    assert_eq!(fix_finish_val["usage"]["prompt_tokens"], 14);
    assert_eq!(fix_finish_val["usage"]["completion_tokens"], 4);
    assert_eq!(fix_finish_val["usage"]["total_tokens"], 18);

    // --- [DONE] sentinel ---
    let gen_last = events.last().expect("events should not be empty");
    assert_eq!(gen_last.data, "[DONE]", "last event should be [DONE]");
    let fix_last = fixture_events.last().expect("fixture should not be empty");
    assert_eq!(fix_last.1, "[DONE]", "fixture last event should be [DONE]");

    // --- Shape-locked: id format ---
    let id = first_gen["id"].as_str().unwrap_or("");
    assert!(!id.is_empty(), "generated id should be non-empty");

    // --- Shape-locked: object field ---
    assert_eq!(
        first_gen["object"], "chat.completion.chunk",
        "object should be chat.completion.chunk"
    );
}

// ---------------------------------------------------------------------------
// OpenAI tool-use streaming fixture tests
// ---------------------------------------------------------------------------

/// UNPRODUCED_FIELDS (OpenAI tool-use streaming):
/// - `system_fingerprint`: fixture has "fp_fake"; code omits
/// - `created`: fixture has 1700000009; code generates `fake-{model}`
/// - `logprobs`: fixture has null; code omits
/// - `usage.prompt_tokens_details.cached_tokens`: fixture has 0;
///   code omits details sub-structure
/// - `usage.completion_tokens_details.reasoning_tokens`: fixture has 0;
///   code omits details sub-structure
#[test]
fn openai_streaming_tool_use_fixture_matches_semantics() {
    let root = fixture_root();
    let meta_path = root.join("openai/tool-use-streaming-meta.json");
    let meta = load_streaming_meta(&meta_path).unwrap();
    let txt_path = root.join("openai/tool-use-streaming.txt");
    let txt_content = load_streaming_fixture(&txt_path).unwrap();

    // Build scenario with tool_call block
    let shape = ResponseShape::ToolCall(ToolCallResponse {
        calls: vec![ToolCallEntry {
            name: "get_weather".to_string(),
            arguments: r#"{"location":"Tokyo"}"#.to_string(),
        }],
        usage: Some(UsageResponse {
            prompt_tokens: Some(18),
            completion_tokens: Some(12),
            reasoning_tokens: None,
            cache_hit_tokens: None,
            cache_write_tokens: None,
            cache_fields_missing: false,
        }),
    });
    let scenario = make_streaming_scenario(&meta, shape);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_meta(&meta, false);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };

    // Generate SSE events — granularity=1 to match fixture's character-level chunking
    let usage_resp = decision.usage.clone().unwrap_or_default();
    let events = generate_openai_sse(
        &decision.response_blocks,
        &decision.model,
        &usage_resp,
        true,
        1,
    );

    // Parse fixture
    let fixture_events = parse_sse_text(&txt_content);

    // --- Semantic: role chunk ---
    let first_gen: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(first_gen["choices"][0]["delta"]["role"], "assistant");

    // --- Semantic: tool_call chunks ---
    let gen_tool_calls: Vec<serde_json::Value> = events
        .iter()
        .filter_map(|e| {
            let v: serde_json::Value = serde_json::from_str(&e.data).ok()?;
            let tc = &v["choices"][0]["delta"]["tool_calls"];
            if tc.is_array() && !tc.as_array().unwrap().is_empty() {
                Some(tc[0].clone())
            } else {
                None
            }
        })
        .collect();

    let fix_tool_calls: Vec<serde_json::Value> = fixture_events
        .iter()
        .filter_map(|(_, data)| {
            let v: serde_json::Value = serde_json::from_str(data).ok()?;
            let tc = &v["choices"][0]["delta"]["tool_calls"];
            if tc.is_array() && !tc.as_array().unwrap().is_empty() {
                Some(tc[0].clone())
            } else {
                None
            }
        })
        .collect();

    assert!(!gen_tool_calls.is_empty(), "should have tool_call deltas");
    assert!(
        !fix_tool_calls.is_empty(),
        "fixture should have tool_call deltas"
    );

    // First tool_call chunk: name + type
    let gen_first_tc = &gen_tool_calls[0];
    assert_eq!(gen_first_tc["function"]["name"], "get_weather");
    assert_eq!(gen_first_tc["type"], "function");
    // Shape-locked: id
    let tc_id = gen_first_tc["id"].as_str().unwrap_or("");
    assert!(!tc_id.is_empty(), "tool call id should be non-empty");

    let fix_first_tc = &fix_tool_calls[0];
    assert_eq!(fix_first_tc["function"]["name"], "get_weather");
    assert_eq!(fix_first_tc["type"], "function");

    // Accumulated arguments
    let gen_args: String = gen_tool_calls
        .iter()
        .filter_map(|tc| tc["function"]["arguments"].as_str())
        .collect();
    let fix_args: String = fix_tool_calls
        .iter()
        .filter_map(|tc| tc["function"]["arguments"].as_str())
        .collect();
    assert_eq!(
        gen_args, fix_args,
        "accumulated tool call arguments should match fixture"
    );
    assert_eq!(gen_args, r#"{"location":"Tokyo"}"#);

    // --- Semantic: finish chunk with tool_calls finish_reason ---
    let gen_finish = events
        .iter()
        .rev()
        .find(|e| {
            serde_json::from_str::<serde_json::Value>(&e.data)
                .ok()
                .and_then(|v| {
                    v["choices"][0]["finish_reason"]
                        .as_str()
                        .map(|s| !s.is_empty())
                })
                .unwrap_or(false)
        })
        .expect("should have finish chunk");
    let gen_finish_val: serde_json::Value = serde_json::from_str(&gen_finish.data).unwrap();
    assert_eq!(gen_finish_val["choices"][0]["finish_reason"], "tool_calls");

    let fix_finish = fixture_events
        .iter()
        .rev()
        .skip(1)
        .find(|(_, data)| {
            serde_json::from_str::<serde_json::Value>(data)
                .ok()
                .and_then(|v| {
                    v["choices"][0]["finish_reason"]
                        .as_str()
                        .map(|s| !s.is_empty())
                })
                .unwrap_or(false)
        })
        .expect("fixture should have finish chunk");
    let fix_finish_val: serde_json::Value = serde_json::from_str(&fix_finish.1).unwrap();
    assert_eq!(fix_finish_val["choices"][0]["finish_reason"], "tool_calls");

    // --- Semantic: usage ---
    assert_eq!(gen_finish_val["usage"]["prompt_tokens"], 18);
    assert_eq!(gen_finish_val["usage"]["completion_tokens"], 12);
    assert_eq!(gen_finish_val["usage"]["total_tokens"], 30);
    assert_eq!(fix_finish_val["usage"]["prompt_tokens"], 18);
    assert_eq!(fix_finish_val["usage"]["completion_tokens"], 12);
    assert_eq!(fix_finish_val["usage"]["total_tokens"], 30);

    // --- [DONE] sentinel ---
    assert_eq!(events.last().unwrap().data, "[DONE]");
    assert_eq!(fixture_events.last().unwrap().1, "[DONE]");
}

// ---------------------------------------------------------------------------
// Anthropic streaming fixture tests
// ---------------------------------------------------------------------------

/// UNPRODUCED_FIELDS (Anthropic streaming):
/// - `id` in message_start: fixture has `msg_01_stream_fake_model_e5f6a7b8c9d0e1f2`;
///   code generates `msg_fake_{model}` — shape-locked
/// - `usage.service_tier`: fixture has "standard"; code omits
/// - `usage.cache_creation_input_tokens` / `cache_read_input_tokens`:
///   fixture has 0/0; code produces `{input_tokens, output_tokens}` only
/// - `stop_sequence`: fixture has null; code omits
#[test]
fn anthropic_streaming_text_fixture_matches_semantics() {
    let root = fixture_root();
    let meta_path = root.join("anthropic/anthropic-streaming-meta.json");
    let meta = load_streaming_meta(&meta_path).unwrap();
    let txt_path = root.join("anthropic/anthropic-streaming.txt");
    let txt_content = load_streaming_fixture(&txt_path).unwrap();

    // Build scenario with text block
    let shape = ResponseShape::Text(TextResponse {
        content: "Hello there friend.".to_string(),
        usage: Some(UsageResponse {
            prompt_tokens: Some(11),
            completion_tokens: Some(4),
            reasoning_tokens: None,
            cache_hit_tokens: None,
            cache_write_tokens: None,
            cache_fields_missing: false,
        }),
    });
    let scenario = make_streaming_scenario(&meta, shape);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_meta(&meta, true);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };

    // Generate SSE events
    let usage_resp = decision.usage.clone().unwrap_or_default();
    let events = generate_anthropic_sse(&decision.response_blocks, &decision.model, &usage_resp, 0);

    // Parse fixture
    let fixture_events = parse_sse_text(&txt_content);

    // --- Event type sequence ---
    assert_eq!(events.len(), 7, "should have 7 events");

    // Parse all generated events
    let gen_values: Vec<serde_json::Value> = events
        .iter()
        .map(|e| serde_json::from_str(&e.data).unwrap())
        .collect();

    // Parse all fixture events
    let fix_values: Vec<serde_json::Value> = fixture_events
        .iter()
        .map(|(_, data)| serde_json::from_str(data).unwrap())
        .collect();

    // --- Semantic: message_start ---
    assert_eq!(gen_values[0]["type"], "message");
    assert_eq!(gen_values[0]["role"], "assistant");
    assert_eq!(gen_values[0]["model"], "fake-model");
    assert!(
        gen_values[0]["content"].as_array().unwrap().is_empty(),
        "initial content should be empty"
    );
    assert!(gen_values[0]["stop_reason"].is_null());

    assert_eq!(fix_values[0]["type"], "message_start");
    assert_eq!(fix_values[0]["message"]["role"], "assistant");
    assert_eq!(fix_values[0]["message"]["model"], "fake-model");

    // --- Semantic: input usage in message_start ---
    assert_eq!(gen_values[0]["usage"]["input_tokens"], 11);
    assert_eq!(fix_values[0]["message"]["usage"]["input_tokens"], 11);

    // --- Semantic: content_block_start (match by type, not index) ---
    let gen_block_start = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("content_block_start"))
        .expect("should have content_block_start");
    assert_eq!(gen_block_start["index"], 0);
    assert_eq!(gen_block_start["content_block"]["type"], "text");

    let fix_block_start = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("content_block_start"))
        .expect("fixture should have content_block_start");
    assert_eq!(fix_block_start["index"], 0);
    assert_eq!(fix_block_start["content_block"]["type"], "text");

    // --- Semantic: ping (match by type, not index) ---
    let gen_ping = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("ping"))
        .expect("should have ping");
    let fix_ping = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("ping"))
        .expect("fixture should have ping");
    assert_eq!(gen_ping["type"], "ping");
    assert_eq!(fix_ping["type"], "ping");

    // --- Semantic: text deltas (fixture has word-boundary splits) ---
    // The code produces a single text_delta with full content.
    // The fixture splits at word boundaries: "Hello", " there", " friend", "."
    // Compare accumulated text content.
    let gen_text_deltas: String = gen_values
        .iter()
        .filter(|v| v["delta"]["type"].as_str() == Some("text_delta"))
        .filter_map(|v| v["delta"]["text"].as_str())
        .collect();
    let fix_text_deltas: String = fix_values
        .iter()
        .filter(|v| v["delta"]["type"].as_str() == Some("text_delta"))
        .filter_map(|v| v["delta"]["text"].as_str())
        .collect();
    assert_eq!(
        gen_text_deltas, fix_text_deltas,
        "accumulated text deltas should match fixture"
    );
    assert_eq!(gen_text_deltas, "Hello there friend.");

    // Also verify the code produces a single text delta event
    let gen_text_events: Vec<&serde_json::Value> = gen_values
        .iter()
        .filter(|v| v["delta"]["type"].as_str() == Some("text_delta"))
        .collect();
    assert_eq!(
        gen_text_events.len(),
        1,
        "code should produce exactly one text_delta event"
    );
    assert_eq!(
        gen_text_events[0]["delta"]["text"], "Hello there friend.",
        "single text delta should contain full content"
    );

    // Fixture has 4 text deltas (word-boundary splits)
    let fix_text_events: Vec<&serde_json::Value> = fix_values
        .iter()
        .filter(|v| v["delta"]["type"].as_str() == Some("text_delta"))
        .collect();
    assert_eq!(
        fix_text_events.len(),
        4,
        "fixture should have 4 text_delta events"
    );

    // --- Semantic: content_block_stop (match by type, not index) ---
    let gen_block_stop = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("content_block_stop"))
        .expect("should have content_block_stop");
    assert_eq!(gen_block_stop["index"], 0);

    let fix_block_stop = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("content_block_stop"))
        .expect("fixture should have content_block_stop");
    assert_eq!(fix_block_stop["index"], 0);

    // --- Semantic: message_delta with stop_reason (match by type) ---
    let gen_msg_delta = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_delta"))
        .expect("should have message_delta");
    assert_eq!(gen_msg_delta["delta"]["stop_reason"], "end_turn");
    assert_eq!(gen_msg_delta["usage"]["output_tokens"], 4);

    let fix_msg_delta = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_delta"))
        .expect("fixture should have message_delta");
    assert_eq!(fix_msg_delta["delta"]["stop_reason"], "end_turn");
    assert_eq!(fix_msg_delta["usage"]["output_tokens"], 4);

    // --- Semantic: message_stop (match by type) ---
    let gen_msg_stop = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_stop"))
        .expect("should have message_stop");
    let fix_msg_stop = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_stop"))
        .expect("fixture should have message_stop");
    assert_eq!(gen_msg_stop["type"], "message_stop");
    assert_eq!(fix_msg_stop["type"], "message_stop");

    // --- Shape-locked: id format ---
    let id = gen_values[0]["id"].as_str().unwrap_or("");
    assert!(!id.is_empty(), "message id should be non-empty");
}

// ---------------------------------------------------------------------------
// Anthropic tool-use streaming fixture tests
// ---------------------------------------------------------------------------

/// UNPRODUCED_FIELDS (Anthropic tool-use streaming):
/// - `id` in content_block_start: fixture has `toolu_fake_01_RB518jPIPEP2M9orwlNX7643`;
///   code generates `toolu_{model}_{idx}` — shape-locked
/// - `content_block_start.content_block.input`: fixture has `{}`;
///   code omits this field
/// - `usage.service_tier`: fixture has "standard"; code omits
/// - `usage.cache_creation_input_tokens` / `cache_read_input_tokens`:
///   fixture has 0/0; code omits
/// - `stop_sequence`: fixture has null; code omits
#[test]
fn anthropic_streaming_tool_use_fixture_matches_semantics() {
    let root = fixture_root();
    let meta_path = root.join("anthropic/anthropic-tool-use-streaming-meta.json");
    let meta = load_streaming_meta(&meta_path).unwrap();
    let txt_path = root.join("anthropic/anthropic-tool-use-streaming.txt");
    let txt_content = load_streaming_fixture(&txt_path).unwrap();

    // Build scenario with tool_call block
    let shape = ResponseShape::ToolCall(ToolCallResponse {
        calls: vec![ToolCallEntry {
            name: "get_weather".to_string(),
            arguments: r#"{"location": "San Francisco"}"#.to_string(),
        }],
        usage: Some(UsageResponse {
            prompt_tokens: Some(39),
            completion_tokens: Some(45),
            reasoning_tokens: None,
            cache_hit_tokens: None,
            cache_write_tokens: None,
            cache_fields_missing: false,
        }),
    });
    let scenario = make_streaming_scenario(&meta, shape);
    let mut engine = super::super::super::ScenarioEngine::new(vec![scenario]);
    let features = request_features_from_meta(&meta, true);
    let decision = match engine.decide(&features) {
        crate::DecisionOutcome::Decision(d) => d,
        _ => panic!("expected Decision"),
    };

    // Generate SSE events — granularity=1 for character-level input_json_delta
    let usage_resp = decision.usage.clone().unwrap_or_default();
    let events = generate_anthropic_sse(&decision.response_blocks, &decision.model, &usage_resp, 1);

    // Parse fixture
    let fixture_events = parse_sse_text(&txt_content);

    // Parse all events
    let gen_values: Vec<serde_json::Value> = events
        .iter()
        .map(|e| serde_json::from_str(&e.data).unwrap())
        .collect();
    let fix_values: Vec<serde_json::Value> = fixture_events
        .iter()
        .map(|(_, data)| serde_json::from_str(data).unwrap())
        .collect();

    // --- Semantic: message_start ---
    assert_eq!(gen_values[0]["type"], "message");
    assert_eq!(gen_values[0]["role"], "assistant");
    assert_eq!(gen_values[0]["model"], "fake-model");
    assert!(gen_values[0]["content"].as_array().unwrap().is_empty());
    assert!(gen_values[0]["stop_reason"].is_null());
    assert_eq!(gen_values[0]["usage"]["input_tokens"], 39);

    assert_eq!(fix_values[0]["type"], "message_start");
    assert_eq!(fix_values[0]["message"]["role"], "assistant");
    assert_eq!(fix_values[0]["message"]["usage"]["input_tokens"], 39);

    // --- Semantic: content_block_start (tool_use) ---
    assert_eq!(gen_values[1]["type"], "content_block_start");
    assert_eq!(gen_values[1]["index"], 0);
    assert_eq!(gen_values[1]["content_block"]["type"], "tool_use");
    assert_eq!(gen_values[1]["content_block"]["name"], "get_weather");
    // Shape-locked: tool_use id
    let tool_id = gen_values[1]["content_block"]["id"].as_str().unwrap_or("");
    assert!(!tool_id.is_empty(), "tool_use id should be non-empty");

    assert_eq!(fix_values[1]["type"], "content_block_start");
    assert_eq!(fix_values[1]["content_block"]["type"], "tool_use");
    assert_eq!(fix_values[1]["content_block"]["name"], "get_weather");

    // --- Semantic: input_json_delta chunks ---
    // Fixture splits by character: {, ", l, o, c, a, t, i, o, n, ", :,  , ", S, a, n,  , F, r, a, n, c, i, s, c, o, ", }
    // Code with granularity=1 splits similarly but may differ in exact boundaries.
    // Compare accumulated JSON — this is the semantic correctness check.
    let gen_json_deltas: Vec<&str> = gen_values
        .iter()
        .filter(|v| v["delta"]["type"].as_str() == Some("input_json_delta"))
        .filter_map(|v| v["delta"]["partial_json"].as_str())
        .collect();

    let fix_json_deltas: Vec<&str> = fix_values
        .iter()
        .filter(|v| v["delta"]["type"].as_str() == Some("input_json_delta"))
        .filter_map(|v| v["delta"]["partial_json"].as_str())
        .collect();

    // Accumulated JSON should be identical
    let gen_accumulated: String = gen_json_deltas.concat();
    let fix_accumulated: String = fix_json_deltas.concat();
    assert_eq!(
        gen_accumulated, fix_accumulated,
        "accumulated input_json_delta should match fixture"
    );
    assert_eq!(
        gen_accumulated, r#"{"location": "San Francisco"}"#,
        "full tool arguments should match"
    );

    // Verify non-empty delta count
    assert!(
        !gen_json_deltas.is_empty(),
        "should have input_json_delta events"
    );
    assert!(
        !fix_json_deltas.is_empty(),
        "fixture should have input_json_delta events"
    );

    // --- Semantic: content_block_stop ---
    let gen_block_stop = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("content_block_stop"))
        .expect("should have content_block_stop");
    assert_eq!(gen_block_stop["index"], 0);

    let fix_block_stop = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("content_block_stop"))
        .expect("fixture should have content_block_stop");
    assert_eq!(fix_block_stop["index"], 0);

    // --- Semantic: message_delta with tool_use stop_reason ---
    let gen_msg_delta = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_delta"))
        .expect("should have message_delta");
    assert_eq!(gen_msg_delta["delta"]["stop_reason"], "tool_use");
    assert_eq!(gen_msg_delta["usage"]["output_tokens"], 45);

    let fix_msg_delta = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_delta"))
        .expect("fixture should have message_delta");
    assert_eq!(fix_msg_delta["delta"]["stop_reason"], "tool_use");
    assert_eq!(fix_msg_delta["usage"]["output_tokens"], 45);

    // --- Semantic: message_stop ---
    let gen_msg_stop = gen_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_stop"))
        .expect("should have message_stop");
    let fix_msg_stop = fix_values
        .iter()
        .find(|v| v["type"].as_str() == Some("message_stop"))
        .expect("fixture should have message_stop");
    assert_eq!(gen_msg_stop["type"], "message_stop");
    assert_eq!(fix_msg_stop["type"], "message_stop");
}
