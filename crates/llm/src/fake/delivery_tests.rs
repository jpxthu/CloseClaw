use std::time::Duration;

use super::super::fake_scenario::{ErrorInjection, StreamInterrupt};
use super::*;

// ── split_segments ───────────────────────────────────────────────────

#[test]
fn test_split_segments_zero_granularity() {
    let segs = split_segments("hello", 0);
    assert_eq!(segs, vec!["hello"]);
}

#[test]
fn test_split_segments_exact_fit() {
    let segs = split_segments("abcde", 5);
    assert_eq!(segs, vec!["abcde"]);
}

#[test]
fn test_split_segments_even_split() {
    let segs = split_segments("abcdef", 3);
    assert_eq!(segs, vec!["abc", "def"]);
}

#[test]
fn test_split_segments_uneven_split() {
    let segs = split_segments("abcde", 2);
    assert_eq!(segs, vec!["ab", "cd", "e"]);
}

#[test]
fn test_split_segments_granularity_one() {
    let segs = split_segments("abc", 1);
    assert_eq!(segs, vec!["a", "b", "c"]);
}

#[test]
fn test_split_segments_empty_content() {
    let segs = split_segments("", 5);
    assert_eq!(segs, vec![""]);
}

// ── OpenAI SSE generation ────────────────────────────────────────────

#[test]
fn test_openai_sse_basic() {
    let usage = RawUsage {
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: Some(30),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(
        &[RawContentBlock::Text("hi".into())],
        "gpt-4",
        &usage,
        false,
    );

    // role chunk → content delta → finish → [DONE] = 4
    assert_eq!(events.len(), 4);

    // Role chunk has assistant role
    let role_data: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(role_data["choices"][0]["delta"]["role"], "assistant");
    assert!(role_data["choices"][0]["delta"]["content"].is_null());

    // Content delta
    let content_data: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(content_data["choices"][0]["delta"]["content"], "hi");

    // Finish chunk
    let finish_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "stop");

    // [DONE]
    assert_eq!(events[3].data, "[DONE]");
}

#[test]
fn test_openai_sse_multiple_segments() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(
        &[
            RawContentBlock::Text("hel".into()),
            RawContentBlock::Text("lo ".into()),
            RawContentBlock::Text("wor".into()),
            RawContentBlock::Text("ld".into()),
        ],
        "gpt-4",
        &usage,
        false,
    );

    // role + 4 content + finish + [DONE] = 7
    assert_eq!(events.len(), 7);

    // Verify each content segment
    let content0: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(content0["choices"][0]["delta"]["content"], "hel");

    let content1: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(content1["choices"][0]["delta"]["content"], "lo ");

    let content2: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(content2["choices"][0]["delta"]["content"], "wor");

    let content3: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
    assert_eq!(content3["choices"][0]["delta"]["content"], "ld");
}

#[test]
fn test_openai_sse_include_usage() {
    let usage = RawUsage {
        prompt_tokens: 50,
        completion_tokens: 100,
        total_tokens: Some(150),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(&[RawContentBlock::Text("a".into())], "gpt-4", &usage, true);

    let finish_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(finish_data["usage"]["prompt_tokens"], 50);
    assert_eq!(finish_data["usage"]["completion_tokens"], 100);
    assert_eq!(finish_data["usage"]["total_tokens"], 150);
}

#[test]
fn test_openai_sse_exclude_usage() {
    let usage = RawUsage {
        prompt_tokens: 50,
        completion_tokens: 100,
        total_tokens: Some(150),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(&[RawContentBlock::Text("a".into())], "gpt-4", &usage, false);

    let finish_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert!(finish_data.get("usage").is_none());
}

#[test]
fn test_openai_sse_usage_computed_when_total_none() {
    let usage = RawUsage {
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(&[RawContentBlock::Text("x".into())], "m", &usage, true);

    let finish_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(finish_data["usage"]["total_tokens"], 30);
}

// ── Anthropic SSE generation ─────────────────────────────────────────

#[test]
fn test_anthropic_sse_basic() {
    let usage = RawUsage {
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: Some(30),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events =
        generate_anthropic_sse(&[RawContentBlock::Text("hello".into())], "claude-3", &usage);

    // message_start + content_block_start + ping + delta + content_block_stop
    // + message_delta + message_stop = 7
    assert_eq!(events.len(), 7);

    // message_start
    let start: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(start["type"], "message");
    assert_eq!(start["role"], "assistant");
    assert_eq!(start["model"], "claude-3");
    assert_eq!(start["usage"]["input_tokens"], 10);

    // ping
    let ping: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(ping["type"], "ping");

    // content_block_start
    let cbs: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(cbs["type"], "content_block_start");
    assert_eq!(cbs["content_block"]["type"], "text");

    // content delta
    let delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(delta["type"], "content_block_delta");
    assert_eq!(delta["delta"]["type"], "text_delta");
    assert_eq!(delta["delta"]["text"], "hello");

    // content_block_stop
    let cbs_end: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
    assert_eq!(cbs_end["type"], "content_block_stop");

    // message_delta
    let msg_delta: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    assert_eq!(msg_delta["type"], "message_delta");
    assert_eq!(msg_delta["delta"]["stop_reason"], "end_turn");
    assert_eq!(msg_delta["usage"]["output_tokens"], 20);

    // message_stop
    let stop: serde_json::Value = serde_json::from_str(&events[6].data).unwrap();
    assert_eq!(stop["type"], "message_stop");
}

#[test]
fn test_anthropic_sse_multiple_segments() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_anthropic_sse(
        &[
            RawContentBlock::Text("ab".into()),
            RawContentBlock::Text("cd".into()),
            RawContentBlock::Text("e".into()),
        ],
        "claude-3",
        &usage,
    );

    // start + ping + 3*(block_start+delta+block_stop) + msg_delta + msg_stop = 13
    assert_eq!(events.len(), 13);

    // Each Text block gets its own content_block_start/delta/stop
    // Block 0 ("ab")
    let d0: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(d0["delta"]["text"], "ab");

    // Block 1 ("cd")
    let d1: serde_json::Value = serde_json::from_str(&events[6].data).unwrap();
    assert_eq!(d1["delta"]["text"], "cd");

    // Block 2 ("e")
    let d2: serde_json::Value = serde_json::from_str(&events[9].data).unwrap();
    assert_eq!(d2["delta"]["text"], "e");
}

#[test]
fn test_anthropic_sse_empty_content() {
    let usage = RawUsage {
        prompt_tokens: 1,
        completion_tokens: 0,
        total_tokens: Some(1),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_anthropic_sse(&[RawContentBlock::Text("".into())], "claude-3", &usage);

    // Still generates full sequence (with empty delta)
    assert_eq!(events.len(), 7);
    let delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(delta["delta"]["text"], "");
}

#[test]
fn test_anthropic_sse_always_includes_usage() {
    let usage = RawUsage {
        prompt_tokens: 100,
        completion_tokens: 200,
        total_tokens: Some(300),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_anthropic_sse(&[RawContentBlock::Text("x".into())], "m", &usage);

    // message_start includes input usage
    let start: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(start["usage"]["input_tokens"], 100);

    // message_delta includes output usage
    let msg_delta: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    assert_eq!(msg_delta["usage"]["output_tokens"], 200);
}

// ── Cross-protocol consistency ───────────────────────────────────────

#[test]
fn test_both_protocols_end_with_stop() {
    let usage = RawUsage {
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: Some(2),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let oai = generate_openai_sse(&[RawContentBlock::Text("a".into())], "m", &usage, false);
    let ant = generate_anthropic_sse(&[RawContentBlock::Text("a".into())], "m", &usage);

    // OpenAI ends with [DONE]
    assert_eq!(oai.last().unwrap().data, "[DONE]");
    // Anthropic ends with message_stop
    let stop: serde_json::Value = serde_json::from_str(&ant.last().unwrap().data).unwrap();
    assert_eq!(stop["type"], "message_stop");
}

#[test]
fn test_all_events_use_message_event_type() {
    let usage = RawUsage {
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: Some(2),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let oai = generate_openai_sse(&[RawContentBlock::Text("a".into())], "m", &usage, false);
    let ant = generate_anthropic_sse(&[RawContentBlock::Text("a".into())], "m", &usage);

    for e in &oai {
        assert_eq!(e.event_type, "message");
    }
    for e in &ant {
        assert_eq!(e.event_type, "message");
    }
}

// ── New block-type SSE tests ─────────────────────────────────────────

#[test]
fn test_openai_sse_thinking_only() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(
        &[RawContentBlock::Thinking {
            thinking: "let me think...".into(),
            signature: None,
        }],
        "gpt-4",
        &usage,
        false,
    );

    // role + reasoning_delta + finish(stop) + [DONE] = 4
    assert_eq!(events.len(), 4);

    let role_data: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
    assert_eq!(role_data["choices"][0]["delta"]["role"], "assistant");

    let reasoning_data: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(
        reasoning_data["choices"][0]["delta"]["reasoning_content"],
        "let me think..."
    );
    assert!(reasoning_data["choices"][0]["delta"]["content"].is_null());

    let finish_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "stop");

    assert_eq!(events[3].data, "[DONE]");
}

#[test]
fn test_openai_sse_tool_use_only() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(
        &[RawContentBlock::ToolUse {
            id: "call_1".into(),
            name: "search".into(),
            input: "{\"q\":\"rust\"}".into(),
        }],
        "gpt-4",
        &usage,
        false,
    );

    // role + tool_call_start + tool_call_delta + finish(tool_calls) + [DONE] = 5
    assert_eq!(events.len(), 5);

    let start_data: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(
        start_data["choices"][0]["delta"]["tool_calls"][0]["id"],
        "call_1"
    );
    assert_eq!(
        start_data["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
        "search"
    );

    let delta_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(
        delta_data["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "{\"q\":\"rust\"}"
    );

    let finish_data: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "tool_calls");

    assert_eq!(events[4].data, "[DONE]");
}

#[test]
fn test_openai_sse_mixed_text_and_tool_use() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_openai_sse(
        &[
            RawContentBlock::Text("Here is the result: ".into()),
            RawContentBlock::ToolUse {
                id: "call_2".into(),
                name: "execute".into(),
                input: "{}".into(),
            },
        ],
        "gpt-4",
        &usage,
        false,
    );

    // role + content_delta + tool_call_start + tool_call_delta + finish(tool_calls) + [DONE] = 6
    assert_eq!(events.len(), 6);

    let content_data: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
    assert_eq!(
        content_data["choices"][0]["delta"]["content"],
        "Here is the result: "
    );

    let start_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(
        start_data["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
        "execute"
    );

    let delta_data: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(
        delta_data["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "{}"
    );

    let finish_data: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "tool_calls");

    assert_eq!(events[5].data, "[DONE]");
}

#[test]
fn test_anthropic_sse_thinking_with_signature() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_anthropic_sse(
        &[RawContentBlock::Thinking {
            thinking: "analyzing...".into(),
            signature: Some("sig_abc".into()),
        }],
        "claude-3",
        &usage,
    );

    // start + ping + block_start + thinking_delta + signature_delta + block_stop
    // + message_delta + message_stop = 8
    assert_eq!(events.len(), 8);

    let block_start: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(block_start["content_block"]["type"], "thinking");

    let think_delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(think_delta["delta"]["type"], "thinking_delta");
    assert_eq!(think_delta["delta"]["thinking"], "analyzing...");

    let sig_delta: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
    assert_eq!(sig_delta["delta"]["type"], "signature_delta");
    assert_eq!(sig_delta["delta"]["signature"], "sig_abc");
}

#[test]
fn test_anthropic_sse_tool_use() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_anthropic_sse(
        &[RawContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "calculator".into(),
            input: "{\"expr\":\"1+1\"}".into(),
        }],
        "claude-3",
        &usage,
    );

    // start + ping + block_start + input_json_delta + block_stop
    // + message_delta + message_stop = 7
    assert_eq!(events.len(), 7);

    let block_start: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(block_start["content_block"]["type"], "tool_use");
    assert_eq!(block_start["content_block"]["id"], "tool_1");
    assert_eq!(block_start["content_block"]["name"], "calculator");

    let json_delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(json_delta["delta"]["type"], "input_json_delta");
    assert_eq!(json_delta["delta"]["partial_json"], "{\"expr\":\"1+1\"}");
}

#[test]
fn test_anthropic_sse_mixed_blocks() {
    let usage = RawUsage {
        prompt_tokens: 5,
        completion_tokens: 10,
        total_tokens: Some(15),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let events = generate_anthropic_sse(
        &[
            RawContentBlock::Thinking {
                thinking: "step 1".into(),
                signature: None,
            },
            RawContentBlock::Text("output".into()),
            RawContentBlock::ToolUse {
                id: "call_3".into(),
                name: "fetch".into(),
                input: "{}".into(),
            },
        ],
        "claude-3",
        &usage,
    );

    // start + ping
    // + thinking_block_start + thinking_delta + thinking_block_stop
    // + text_block_start + text_delta + text_block_stop
    // + tool_block_start + input_json_delta + tool_block_stop
    // + message_delta + message_stop = 13
    assert_eq!(events.len(), 13);

    // Thinking block
    let think_start: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
    assert_eq!(think_start["content_block"]["type"], "thinking");
    let think_delta: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
    assert_eq!(think_delta["delta"]["type"], "thinking_delta");

    // Text block
    let text_start: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
    assert_eq!(text_start["content_block"]["type"], "text");
    let text_delta: serde_json::Value = serde_json::from_str(&events[6].data).unwrap();
    assert_eq!(text_delta["delta"]["type"], "text_delta");
    assert_eq!(text_delta["delta"]["text"], "output");

    // ToolUse block
    let tool_start: serde_json::Value = serde_json::from_str(&events[8].data).unwrap();
    assert_eq!(tool_start["content_block"]["type"], "tool_use");
    let tool_delta: serde_json::Value = serde_json::from_str(&events[9].data).unwrap();
    assert_eq!(tool_delta["delta"]["type"], "input_json_delta");
}

// ── Delay injection ─────────────────────────────────────────────────

#[tokio::test]
async fn test_apply_first_token_delay_none() {
    let config = DeliveryConfig::default();
    apply_first_token_delay(&config).await;
    // No-op, completes immediately
}

#[tokio::test]
async fn test_apply_first_token_delay_zero() {
    let config = DeliveryConfig {
        first_token_delay: Some(Duration::ZERO),
        ..Default::default()
    };
    apply_first_token_delay(&config).await;
    // Zero delay is a no-op
}

#[tokio::test]
async fn test_apply_first_token_delay_with_duration() {
    let config = DeliveryConfig {
        first_token_delay: Some(Duration::from_millis(10)),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    apply_first_token_delay(&config).await;
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(10));
}

#[tokio::test]
async fn test_apply_per_segment_delay_none() {
    let config = DeliveryConfig::default();
    apply_per_segment_delay(&config).await;
}

#[tokio::test]
async fn test_apply_per_segment_delay_zero() {
    let config = DeliveryConfig {
        per_segment_delay: Some(Duration::ZERO),
        ..Default::default()
    };
    apply_per_segment_delay(&config).await;
}

#[tokio::test]
async fn test_apply_per_segment_delay_with_duration() {
    let config = DeliveryConfig {
        per_segment_delay: Some(Duration::from_millis(10)),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    apply_per_segment_delay(&config).await;
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(10));
}

#[tokio::test]
async fn test_apply_overall_delay_none() {
    let config = DeliveryConfig::default();
    apply_overall_delay(&config).await;
}

#[tokio::test]
async fn test_apply_overall_delay_zero() {
    let config = DeliveryConfig {
        overall_delay: Some(Duration::ZERO),
        ..Default::default()
    };
    apply_overall_delay(&config).await;
}

#[tokio::test]
async fn test_apply_overall_delay_with_duration() {
    let config = DeliveryConfig {
        overall_delay: Some(Duration::from_millis(10)),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    apply_overall_delay(&config).await;
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(10));
}

// ── Error injection ──────────────────────────────────────────────────

#[test]
fn test_should_inject_http_error_none() {
    let config = DeliveryConfig::default();
    assert!(should_inject_http_error(&config).is_none());
}

#[test]
fn test_should_inject_http_error_401() {
    let config = DeliveryConfig {
        error_injection: Some(ErrorInjection {
            status_code: 401,
            message: "unauthorized".into(),
            retry_after: None,
        }),
        ..Default::default()
    };
    let result = should_inject_http_error(&config).unwrap();
    assert_eq!(result.0, 401);
    assert_eq!(result.1, "unauthorized");
    assert!(result.2.is_none());
}

#[test]
fn test_should_inject_http_error_429_with_retry_after() {
    let config = DeliveryConfig {
        error_injection: Some(ErrorInjection {
            status_code: 429,
            message: "rate limited".into(),
            retry_after: Some(30),
        }),
        ..Default::default()
    };
    let result = should_inject_http_error(&config).unwrap();
    assert_eq!(result.0, 429);
    assert_eq!(result.1, "rate limited");
    assert_eq!(result.2, Some(30));
}

#[test]
fn test_should_inject_http_error_500() {
    let config = DeliveryConfig {
        error_injection: Some(ErrorInjection {
            status_code: 500,
            message: "internal server error".into(),
            retry_after: None,
        }),
        ..Default::default()
    };
    let result = should_inject_http_error(&config).unwrap();
    assert_eq!(result.0, 500);
    assert_eq!(result.1, "internal server error");
}

#[test]
fn test_should_interrupt_stream_none() {
    let config = DeliveryConfig::default();
    assert!(should_interrupt_stream(&config).is_none());
}

#[test]
fn test_should_interrupt_stream_zero() {
    let config = DeliveryConfig {
        stream_interrupt: Some(StreamInterrupt {
            interrupt_after_frames: 0,
        }),
        ..Default::default()
    };
    let result = should_interrupt_stream(&config).unwrap();
    assert_eq!(result, 0);
}

#[test]
fn test_should_interrupt_stream_with_value() {
    let config = DeliveryConfig {
        stream_interrupt: Some(StreamInterrupt {
            interrupt_after_frames: 5,
        }),
        ..Default::default()
    };
    let result = should_interrupt_stream(&config).unwrap();
    assert_eq!(result, 5);
}
