//! SSE event generation for `FakeProvider` delivery layer.
//!
//! Generates protocol-compliant SSE event sequences for both OpenAI and
//! Anthropic protocols. Content is split into segments based on the
//! configured `segment_granularity`.

use crate::types::RawUsage;

/// A single SSE event ready to be sent over the channel.
///
/// Maps directly to [`RawSseChunk`] when consumed by the streaming pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// SSE event type (e.g., `"message"`).
    pub event_type: String,
    /// JSON-serialized event data.
    pub data: String,
}

/// Split `content` into segments of at most `granularity` characters.
///
/// When `granularity` is 0, the entire content is returned as a single segment.
#[allow(dead_code)]
pub(crate) fn split_segments(content: &str, granularity: usize) -> Vec<String> {
    if granularity == 0 || content.len() <= granularity {
        return vec![content.to_string()];
    }
    content
        .chars()
        .collect::<Vec<_>>()
        .chunks(granularity)
        .map(|c| c.iter().collect())
        .collect()
}

/// Generate an OpenAI-compatible SSE event sequence.
///
/// Sequence:
/// 1. `delta.role = "assistant"` — role chunk
/// 2. Content delta chunks (one per segment)
/// 3. Finish chunk with `finish_reason = "stop"` — optionally includes usage
/// 4. `[DONE]` sentinel
#[allow(dead_code)]
pub(crate) fn generate_openai_sse(
    segments: Vec<String>,
    model: &str,
    usage: &RawUsage,
    include_usage: bool,
) -> Vec<SseEvent> {
    let mut events = Vec::with_capacity(segments.len() + 3);
    let id = format!("fake-{}", model);

    // 1. Role chunk
    events.push(SseEvent {
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
    });

    // 2. Content delta chunks
    for segment in &segments {
        events.push(SseEvent {
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
        });
    }

    // 3. Finish chunk (with optional usage)
    let mut finish_value = serde_json::json!({
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
        finish_value["usage"] = serde_json::json!({
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens.unwrap_or(
                usage.prompt_tokens + usage.completion_tokens
            )
        });
    }
    events.push(SseEvent {
        event_type: "message".into(),
        data: finish_value.to_string(),
    });

    // 4. [DONE]
    events.push(SseEvent {
        event_type: "message".into(),
        data: "[DONE]".into(),
    });

    events
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
#[allow(dead_code)]
pub(crate) fn generate_anthropic_sse(
    segments: Vec<String>,
    model: &str,
    usage: &RawUsage,
) -> Vec<SseEvent> {
    let mut events = Vec::with_capacity(segments.len() + 6);
    let msg_id = format!("msg_fake_{}", model);

    // 1. message_start
    events.push(SseEvent {
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
    });

    // 2. content_block_start
    events.push(SseEvent {
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
    });

    // 3. ping
    events.push(SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({"type": "ping"}).to_string(),
    });

    // 4. Content deltas
    for segment in &segments {
        events.push(SseEvent {
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
        });
    }

    // 5. content_block_stop
    events.push(SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_stop",
            "index": 0
        })
        .to_string(),
    });

    // 6. message_delta
    events.push(SseEvent {
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
    });

    // 7. message_stop
    events.push(SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({"type": "message_stop"}).to_string(),
    });

    events
}

#[cfg(test)]
mod tests {
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
}
