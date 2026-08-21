use crate::delivery::sse::*;
use crate::scenario::types::{ResponseBlock, UsageResponse};

mod inject;

fn text_block(content: &str) -> ResponseBlock {
    ResponseBlock {
        block_type: "text".to_string(),
        content: Some(content.to_string()),
        tool_name: None,
        tool_arguments: None,
        reasoning: None,
        signature: None,
    }
}
fn reasoning_block(reasoning: &str, content: &str) -> ResponseBlock {
    ResponseBlock {
        block_type: "reasoning".to_string(),
        content: Some(content.to_string()),
        tool_name: None,
        tool_arguments: None,
        reasoning: Some(reasoning.to_string()),
        signature: None,
    }
}
fn reasoning_block_with_sig(reasoning: &str, content: &str, sig: &str) -> ResponseBlock {
    ResponseBlock {
        block_type: "reasoning".to_string(),
        content: Some(content.to_string()),
        tool_name: None,
        tool_arguments: None,
        reasoning: Some(reasoning.to_string()),
        signature: Some(sig.to_string()),
    }
}
fn tool_call_block(name: &str, args: &str) -> ResponseBlock {
    ResponseBlock {
        block_type: "tool_call".to_string(),
        content: None,
        tool_name: Some(name.to_string()),
        tool_arguments: Some(args.to_string()),
        reasoning: None,
        signature: None,
    }
}
fn default_usage() -> UsageResponse {
    UsageResponse {
        prompt_tokens: Some(10),
        completion_tokens: Some(20),
        reasoning_tokens: None,
        cache_hit_tokens: None,
        cache_write_tokens: None,
        cache_fields_missing: false,
    }
}
fn usage_with(
    prompt: Option<u32>,
    completion: Option<u32>,
    cache_hit: Option<u32>,
    cache_write: Option<u32>,
) -> UsageResponse {
    UsageResponse {
        prompt_tokens: prompt,
        completion_tokens: completion,
        reasoning_tokens: None,
        cache_hit_tokens: cache_hit,
        cache_write_tokens: cache_write,
        cache_fields_missing: false,
    }
}

// split_segments

#[test]
fn split_segments_zero_granularity_returns_whole() {
    let result = split_segments("hello world", 0);
    assert_eq!(result, vec!["hello world"]);
}
#[test]
fn split_segments_longer_than_content() {
    let result = split_segments("hi", 100);
    assert_eq!(result, vec!["hi"]);
}
#[test]
fn split_segments_exact_boundary() {
    let result = split_segments("abcd", 2);
    assert_eq!(result, vec!["ab", "cd"]);
}
#[test]
fn split_segments_with_remainder() {
    let result = split_segments("abcde", 2);
    assert_eq!(result, vec!["ab", "cd", "e"]);
}
#[test]
fn split_segments_unicode() {
    let result = split_segments("你好世界", 2);
    assert_eq!(result, vec!["你好", "世界"]);
}
#[test]
fn split_segments_empty_string() {
    let result = split_segments("", 5);
    assert_eq!(result, vec![""]);
}

// OpenAI SSE

#[test]
fn openai_sse_text_only() {
    let blocks = vec![text_block("Hello!")];
    let usage = default_usage();
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, false, 0);
    assert_eq!(events.len(), 4);
    let role: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(role["choices"][0]["delta"]["role"], "assistant");
    let content: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(content["choices"][0]["delta"]["content"], "Hello!");
    let finish: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(finish["choices"][0]["finish_reason"], "stop");
    assert_eq!(events[3].data, "[DONE]");
}

#[test]
fn openai_sse_with_usage() {
    let blocks = vec![text_block("Hi")];
    let usage = default_usage();
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, true, 0);

    // finish chunk should contain usage
    let finish_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(finish_data["usage"]["prompt_tokens"], 10);
    assert_eq!(finish_data["usage"]["completion_tokens"], 20);
    assert_eq!(finish_data["usage"]["total_tokens"], 30);
}

#[test]
fn openai_sse_reasoning_block() {
    let blocks = vec![reasoning_block("Let me think", "The answer is 42")];
    let usage = default_usage();
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, false, 0);
    assert_eq!(events.len(), 5);
    let r: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(
        r["choices"][0]["delta"]["reasoning_content"],
        "Let me think"
    );
    let c: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(c["choices"][0]["delta"]["content"], "The answer is 42");
}

#[test]
fn openai_sse_tool_call_block() {
    let blocks = vec![tool_call_block("get_weather", r#"{"city":"BJ"}"#)];
    let usage = default_usage();
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, false, 0);
    assert_eq!(events.len(), 5);
    let s: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(
        s["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
        "get_weather"
    );
    let d: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(
        d["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        r#"{"city":"BJ"}"#
    );
    let f: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(f["choices"][0]["finish_reason"], "tool_calls");
}

#[test]
fn openai_sse_tool_call_segmented() {
    let blocks = vec![tool_call_block("search", "abcdef")];
    let usage = default_usage();
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, false, 3);

    // role chunk, tool_call_start, 2 deltas, finish, [DONE]
    assert_eq!(events.len(), 6);
    let delta1: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(
        delta1["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "abc"
    );
    let delta2: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(
        delta2["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "def"
    );
}

#[test]
fn openai_sse_mixed_text_and_tool() {
    let blocks = vec![
        text_block("Let me look that up"),
        tool_call_block("search", r#"{"q":"rust"}"#),
    ];
    let usage = default_usage();
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, false, 0);

    // role, text delta, tool_start, tool_delta, finish(tool_calls), [DONE]
    assert_eq!(events.len(), 6);
    let finish_data: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "tool_calls");
}

// Anthropic SSE

#[test]
fn anthropic_sse_text_only() {
    let blocks = vec![text_block("Hello!")];
    let usage = default_usage();
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    assert_eq!(events.len(), 7);
    let start: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(start["type"], "message");
    assert_eq!(start["role"], "assistant");
    let delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(delta["delta"]["type"], "text_delta");
    assert_eq!(delta["delta"]["text"], "Hello!");
    let msg_delta: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    assert_eq!(msg_delta["delta"]["stop_reason"], "end_turn");
    assert_eq!(msg_delta["usage"]["output_tokens"], 20);
}

#[test]
fn anthropic_sse_reasoning_block() {
    let blocks = vec![reasoning_block("Thinking...", "Done.")];
    let usage = default_usage();
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);

    // message_start, content_block_start(thinking), ping,
    // thinking_delta, content_block_stop, message_delta, message_stop
    assert_eq!(events.len(), 7);

    let block_start: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(block_start["content_block"]["type"], "thinking");

    let delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(delta["delta"]["type"], "thinking_delta");
    assert_eq!(delta["delta"]["thinking"], "Thinking...");
}

#[test]
fn anthropic_sse_reasoning_with_signature() {
    let blocks = vec![reasoning_block_with_sig(
        "reasoning text",
        "answer",
        "sig123",
    )];
    let usage = default_usage();
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);

    // message_start, content_block_start, ping, thinking_delta,
    // signature_delta, content_block_stop, message_delta, message_stop
    assert_eq!(events.len(), 8);

    let sig_delta: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
    assert_eq!(sig_delta["delta"]["type"], "signature_delta");
    assert_eq!(sig_delta["delta"]["signature"], "sig123");
}

#[test]
fn anthropic_sse_tool_call_block() {
    let blocks = vec![tool_call_block("get_weather", r#"{"city":"BJ"}"#)];
    let usage = default_usage();
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);

    // message_start, content_block_start(tool_use), ping,
    // input_json_delta, content_block_stop, message_delta, message_stop
    assert_eq!(events.len(), 7);

    let block_start: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(block_start["content_block"]["type"], "tool_use");
    assert_eq!(block_start["content_block"]["name"], "get_weather");

    let delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(delta["delta"]["type"], "input_json_delta");
    assert_eq!(delta["delta"]["partial_json"], r#"{"city":"BJ"}"#);

    let msg_delta: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    assert_eq!(msg_delta["delta"]["stop_reason"], "tool_use");
}

#[test]
fn anthropic_sse_tool_call_segmented() {
    let blocks = vec![tool_call_block("search", "abcdef")];
    let usage = default_usage();
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 3);
    assert_eq!(events.len(), 8);
    let d1: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(d1["delta"]["partial_json"], "abc");
    let d2: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
    assert_eq!(d2["delta"]["partial_json"], "def");
}

#[test]
fn anthropic_sse_multiple_blocks() {
    let blocks = vec![
        reasoning_block("Thinking...", "Result"),
        text_block("Here is the answer"),
    ];
    let usage = default_usage();
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    assert_eq!(events.len(), 10);
    let b1: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(b1["content_block"]["type"], "thinking");
    let b2: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    assert_eq!(b2["content_block"]["type"], "text");
}

// SseEvent structure

#[test]
fn sse_event_fields_and_clone() {
    let e1 = SseEvent {
        event_type: "msg".into(),
        data: "dat".into(),
    };
    let e2 = e1.clone();
    assert_eq!(e1, e2);
    assert_eq!(e1.event_type, "msg");
    assert_eq!(e1.data, "dat");
}

// KV cache field serialization
#[test]
fn openai_finish_with_cache_hit_tokens() {
    let blocks = vec![text_block("Hi")];
    let usage = usage_with(Some(100), Some(50), Some(50), None);
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, true, 0);
    let d: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(d["usage"]["prompt_tokens_details"]["cached_tokens"], 50);
}

#[test]
fn openai_finish_without_cache_hit_tokens() {
    let blocks = vec![text_block("Hi")];
    let usage = usage_with(Some(100), Some(50), None, None);
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, true, 0);
    let d: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert!(d["usage"]["prompt_tokens_details"].is_null());
}

#[test]
fn openai_finish_cache_hit_zero_omitted() {
    let blocks = vec![text_block("Hi")];
    let usage = usage_with(Some(100), Some(50), Some(0), None);
    let events = generate_openai_sse(&blocks, "gpt-4", &usage, true, 0);
    let d: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert!(d["usage"]["prompt_tokens_details"].is_null());
}

#[test]
fn anthropic_start_includes_cache_fields() {
    let blocks = vec![text_block("Hello!")];
    let usage = usage_with(Some(200), Some(100), Some(150), Some(200));
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    let d: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(d["usage"]["cache_read_input_tokens"], 150);
    assert_eq!(d["usage"]["cache_creation_input_tokens"], 200);
}

#[test]
fn anthropic_start_cache_fields_default_zero() {
    let blocks = vec![text_block("Hello!")];
    let usage = usage_with(Some(200), Some(100), None, None);
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    let d: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(d["usage"]["cache_read_input_tokens"], 0);
    assert_eq!(d["usage"]["cache_creation_input_tokens"], 0);
}

#[test]
fn anthropic_delta_includes_cache_read_tokens() {
    let blocks = vec![text_block("Hello!")];
    let usage = usage_with(Some(200), Some(100), Some(150), None);
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    let d: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    assert_eq!(d["usage"]["cache_read_input_tokens"], 150);
}

#[test]
fn anthropic_start_cache_fields_missing_with_explicit_injection() {
    let blocks = vec![text_block("Hello!")];
    let mut usage = usage_with(Some(200), Some(100), Some(150), Some(200));
    usage.cache_fields_missing = true;
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    let d: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    // Explicit injection overrides cache_fields_missing
    assert_eq!(d["usage"]["cache_read_input_tokens"], 150);
    assert_eq!(d["usage"]["cache_creation_input_tokens"], 200);
}

#[test]
fn anthropic_start_cache_fields_missing_no_explicit_omits_cache_tokens() {
    let blocks = vec![text_block("Hello!")];
    let mut usage = usage_with(Some(200), Some(100), None, None);
    usage.cache_fields_missing = true;
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    let d: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    // cache_fields_missing=true + no explicit values → fields omitted
    assert_eq!(
        d["usage"]["cache_read_input_tokens"],
        serde_json::Value::Null
    );
    assert_eq!(
        d["usage"]["cache_creation_input_tokens"],
        serde_json::Value::Null
    );
}

#[test]
fn anthropic_delta_cache_fields_missing_with_explicit_injection() {
    let blocks = vec![text_block("Hello!")];
    let mut usage = usage_with(Some(200), Some(100), Some(150), None);
    usage.cache_fields_missing = true;
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    let d: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    // Explicit injection overrides cache_fields_missing
    assert_eq!(d["usage"]["cache_read_input_tokens"], 150);
}

#[test]
fn anthropic_delta_cache_fields_missing_no_explicit_omits_cache_tokens() {
    let blocks = vec![text_block("Hello!")];
    let mut usage = usage_with(Some(200), Some(100), None, None);
    usage.cache_fields_missing = true;
    let events = generate_anthropic_sse(&blocks, "claude-3", &usage, 0);
    let d: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    // cache_fields_missing=true + no explicit values → field omitted
    assert_eq!(
        d["usage"]["cache_read_input_tokens"],
        serde_json::Value::Null
    );
}
