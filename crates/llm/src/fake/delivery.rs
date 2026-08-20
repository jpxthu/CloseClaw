//! SSE event generation and injection logic for `FakeProvider` delivery layer.
//!
//! Generates protocol-compliant SSE event sequences for both OpenAI and
//! Anthropic protocols. Content is split into segments based on the
//! configured `segment_granularity`.
//!
//! Also provides delay injection and error injection helpers consumed by
//! [`FakeProvider::send`] and [`FakeProvider::send_streaming`].

use super::fake_scenario::DeliveryConfig;
use crate::types::RawUsage;

/// A single SSE event ready to be sent over the channel.
///
/// Maps directly to [`RawSseChunk`] when consumed by the streaming pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// SSE event type (e.g., `"message"`).
    pub(crate) event_type: String,
    /// JSON-serialized event data.
    pub(crate) data: String,
}

/// Split `content` into segments of at most `granularity` characters.
///
/// When `granularity` is 0, the entire content is returned as a single segment.
pub(crate) fn split_segments(content: &str, granularity: usize) -> Vec<String> {
    if granularity == 0 || content.chars().count() <= granularity {
        return vec![content.to_string()];
    }
    content
        .chars()
        .collect::<Vec<_>>()
        .chunks(granularity)
        .map(|c| c.iter().collect())
        .collect()
}

/// OpenAI role chunk event.
fn openai_role_chunk(model: &str) -> SseEvent {
    let id = format!("fake-{}", model);
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "id": id,
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {"role": "assistant"},
                "finish_reason": null
            }]
        })
        .to_string(),
    }
}

/// OpenAI content delta event.
fn openai_content_delta(model: &str, segment: &str) -> SseEvent {
    let id = format!("fake-{}", model);
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "id": id,
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {"content": segment},
                "finish_reason": null
            }]
        })
        .to_string(),
    }
}

/// OpenAI finish chunk event with optional usage.
fn openai_finish_chunk(model: &str, usage: &RawUsage, include_usage: bool) -> SseEvent {
    let id = format!("fake-{}", model);
    let mut value = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    });
    if include_usage {
        value["usage"] = serde_json::json!({
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens.unwrap_or(
                usage.prompt_tokens + usage.completion_tokens
            )
        });
    }
    SseEvent {
        event_type: "message".into(),
        data: value.to_string(),
    }
}

/// OpenAI [DONE] sentinel event.
fn openai_done() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: "[DONE]".into(),
    }
}

/// Generate an OpenAI-compatible SSE event sequence.
///
/// Sequence:
/// 1. `delta.role = "assistant"` — role chunk
/// 2. Content delta chunks (one per segment)
/// 3. Finish chunk with `finish_reason = "stop"` — optionally includes usage
/// 4. `[DONE]` sentinel
pub(crate) fn generate_openai_sse(
    segments: Vec<String>,
    model: &str,
    usage: &RawUsage,
    include_usage: bool,
) -> Vec<SseEvent> {
    let mut events = Vec::with_capacity(segments.len() + 3);
    events.push(openai_role_chunk(model));
    for segment in &segments {
        events.push(openai_content_delta(model, segment));
    }
    events.push(openai_finish_chunk(model, usage, include_usage));
    events.push(openai_done());
    events
}

/// Anthropic message_start event.
fn anthropic_message_start(model: &str, usage: &RawUsage) -> SseEvent {
    let msg_id = format!("msg_fake_{}", model);
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "message",
            "id": msg_id,
            "role": "assistant",
            "content": [],
            "model": model,
            "stop_reason": null,
            "usage": {
                "input_tokens": usage.prompt_tokens,
                "output_tokens": 0
            }
        })
        .to_string(),
    }
}

/// Anthropic content_block_start event.
fn anthropic_content_block_start() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "text",
                "text": ""
            }
        })
        .to_string(),
    }
}

/// Anthropic ping event.
fn anthropic_ping() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({"type": "ping"}).to_string(),
    }
}

/// Anthropic content_block_delta event.
fn anthropic_content_delta(segment: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "text_delta",
                "text": segment
            }
        })
        .to_string(),
    }
}

/// Anthropic content_block_stop event.
fn anthropic_content_block_stop() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_stop",
            "index": 0
        })
        .to_string(),
    }
}

/// Anthropic message_delta event with stop reason and output usage.
fn anthropic_message_delta(usage: &RawUsage) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": "end_turn",
                "stop_sequence": null
            },
            "usage": {
                "output_tokens": usage.completion_tokens
            }
        })
        .to_string(),
    }
}

/// Anthropic message_stop event.
fn anthropic_message_stop() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({"type": "message_stop"}).to_string(),
    }
}

/// Generate an Anthropic-compatible SSE event sequence.
///
/// Sequence:
/// 1. `message_start` — model name + initial input usage
/// 2. `content_block_start` — type "text"
/// 3. `ping`
/// 4. Content delta chunks (one per segment, `text_delta`)
/// 5. `content_block_stop`
/// 6. `message_delta` — stop_reason + final output usage
/// 7. `message_stop`
pub(crate) fn generate_anthropic_sse(
    segments: Vec<String>,
    model: &str,
    usage: &RawUsage,
) -> Vec<SseEvent> {
    let mut events = Vec::with_capacity(segments.len() + 6);
    events.push(anthropic_message_start(model, usage));
    events.push(anthropic_content_block_start());
    events.push(anthropic_ping());
    for segment in &segments {
        events.push(anthropic_content_delta(segment));
    }
    events.push(anthropic_content_block_stop());
    events.push(anthropic_message_delta(usage));
    events.push(anthropic_message_stop());
    events
}

// ── Delay injection ─────────────────────────────────────────────────────

/// Sleep for the configured first-token delay.
///
/// Called once before the first SSE frame is emitted in a streaming
/// response. No-op when `first_token_delay` is `None` or zero.
pub(crate) async fn apply_first_token_delay(config: &DeliveryConfig) {
    if let Some(delay) = config.first_token_delay {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
}

/// Sleep for the configured per-segment delay.
///
/// Called between consecutive SSE frames during streaming. No-op when
/// `per_segment_delay` is `None` or zero.
pub(crate) async fn apply_per_segment_delay(config: &DeliveryConfig) {
    if let Some(delay) = config.per_segment_delay {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
}

/// Sleep for the configured overall delay (non-streaming only).
///
/// Called before returning the complete response in the non-streaming
/// path. No-op when `overall_delay` is `None` or zero.
pub(crate) async fn apply_overall_delay(config: &DeliveryConfig) {
    if let Some(delay) = config.overall_delay {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
}

// ── Error injection ───────────────────────────────────────────────────────

/// Check if an HTTP error should be injected.
///
/// Returns `Some((status_code, body, retry_after))` when the delivery
/// config contains an error injection, `None` otherwise.
pub(crate) fn should_inject_http_error(
    config: &DeliveryConfig,
) -> Option<(u16, String, Option<u64>)> {
    config
        .error_injection
        .as_ref()
        .map(|ei| (ei.status_code, ei.message.clone(), ei.retry_after))
}

/// Check if the stream should be interrupted after a given number of frames.
///
/// Returns `Some(interrupt_after_frames)` when stream interruption is
/// configured, `None` otherwise.
pub(crate) fn should_interrupt_stream(config: &DeliveryConfig) -> Option<usize> {
    config
        .stream_interrupt
        .as_ref()
        .map(|si| si.interrupt_after_frames)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        let events = generate_openai_sse(vec!["hi".into()], "gpt-4", &usage, false);

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
            vec!["hel".into(), "lo ".into(), "wor".into(), "ld".into()],
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
        let events = generate_openai_sse(vec!["a".into()], "gpt-4", &usage, true);

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
        let events = generate_openai_sse(vec!["a".into()], "gpt-4", &usage, false);

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
        let events = generate_openai_sse(vec!["x".into()], "m", &usage, true);

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
        let events = generate_anthropic_sse(vec!["hello".into()], "claude-3", &usage);

        // message_start + content_block_start + ping + delta + content_block_stop
        // + message_delta + message_stop = 7
        assert_eq!(events.len(), 7);

        // message_start
        let start: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
        assert_eq!(start["type"], "message");
        assert_eq!(start["role"], "assistant");
        assert_eq!(start["model"], "claude-3");
        assert_eq!(start["usage"]["input_tokens"], 10);

        // content_block_start
        let cbs: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
        assert_eq!(cbs["type"], "content_block_start");
        assert_eq!(cbs["content_block"]["type"], "text");

        // ping
        let ping: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
        assert_eq!(ping["type"], "ping");

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
            vec!["ab".into(), "cd".into(), "e".into()],
            "claude-3",
            &usage,
        );

        // start + block_start + ping + 3 deltas + block_stop + msg_delta + msg_stop = 9
        assert_eq!(events.len(), 9);

        // First delta
        let d0: serde_json::Value = serde_json::from_str(&events[3].data).unwrap();
        assert_eq!(d0["delta"]["text"], "ab");

        // Second delta
        let d1: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
        assert_eq!(d1["delta"]["text"], "cd");

        // Third delta
        let d2: serde_json::Value = serde_json::from_str(&events[5].data).unwrap();
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
        let events = generate_anthropic_sse(vec!["".into()], "claude-3", &usage);

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
        let events = generate_anthropic_sse(vec!["x".into()], "m", &usage);

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
        let oai = generate_openai_sse(vec!["a".into()], "m", &usage, false);
        let ant = generate_anthropic_sse(vec!["a".into()], "m", &usage);

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
        let oai = generate_openai_sse(vec!["a".into()], "m", &usage, false);
        let ant = generate_anthropic_sse(vec!["a".into()], "m", &usage);

        for e in &oai {
            assert_eq!(e.event_type, "message");
        }
        for e in &ant {
            assert_eq!(e.event_type, "message");
        }
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
}
