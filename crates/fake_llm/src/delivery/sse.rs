//! SSE event generation for the Fake LLM Server delivery layer.
//!
//! Generates protocol-compliant SSE event sequences for both OpenAI and
//! Anthropic protocols. Content is split into segments based on the
//! configured `segment_granularity`.
//!
//! See `docs/design/fake_llm/delivery.md` for the full specification.

use crate::scenario::types::{ResponseBlock, UsageResponse};

// ---------------------------------------------------------------------------
// SseEvent type
// ---------------------------------------------------------------------------

/// A single SSE event ready to be sent over the channel.
///
/// Maps directly to an axum SSE response chunk when consumed by the
/// endpoint handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// SSE event type (e.g., `"message"`).
    pub event_type: String,
    /// JSON-serialized event data.
    pub data: String,
}

// ---------------------------------------------------------------------------
// Content segmentation
// ---------------------------------------------------------------------------

/// Split `content` into segments of at most `granularity` characters.
///
/// When `granularity` is 0, the entire content is returned as a single segment.
pub fn split_segments(content: &str, granularity: usize) -> Vec<String> {
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

// ---------------------------------------------------------------------------
// OpenAI SSE helpers
// ---------------------------------------------------------------------------

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

fn openai_reasoning_delta(model: &str, text: &str) -> SseEvent {
    let id = format!("fake-{}", model);
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "id": id,
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {"reasoning_content": text},
                "finish_reason": null
            }]
        })
        .to_string(),
    }
}

fn openai_tool_call_start(model: &str, id: &str, name: &str) -> SseEvent {
    let msg_id = format!("fake-{}", model);
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "id": msg_id,
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": ""
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
    }
}

fn openai_tool_call_delta(model: &str, id: &str, arguments: &str) -> SseEvent {
    let msg_id = format!("fake-{}", model);
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "id": msg_id,
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": id,
                        "function": {
                            "arguments": arguments
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
    }
}

fn openai_finish_chunk(
    model: &str,
    usage: &UsageResponse,
    include_usage: bool,
    finish_reason: &str,
) -> SseEvent {
    let id = format!("fake-{}", model);
    let mut value = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": finish_reason
        }]
    });
    if include_usage {
        let prompt = usage.prompt_tokens.unwrap_or(0);
        let completion = usage.completion_tokens.unwrap_or(0);
        let mut usage_json = serde_json::json!({
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "total_tokens": prompt + completion
        });
        if let Some(n) = usage.cache_hit_tokens {
            if n > 0 {
                usage_json["prompt_tokens_details"] = serde_json::json!({ "cached_tokens": n });
            }
        }
        value["usage"] = usage_json;
    }
    SseEvent {
        event_type: "message".into(),
        data: value.to_string(),
    }
}

fn openai_done() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: "[DONE]".into(),
    }
}

// ---------------------------------------------------------------------------
// Anthropic SSE helpers
// ---------------------------------------------------------------------------

fn anthropic_message_start(model: &str, usage: &UsageResponse) -> SseEvent {
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
                "input_tokens": usage.prompt_tokens.unwrap_or(0),
                "output_tokens": 0,
                "cache_read_input_tokens": usage.cache_hit_tokens.unwrap_or(0),
                "cache_creation_input_tokens": usage.cache_write_tokens.unwrap_or(0)
            }
        })
        .to_string(),
    }
}

fn anthropic_text_content_block_start(index: usize) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {
                "type": "text",
                "text": ""
            }
        })
        .to_string(),
    }
}

fn anthropic_thinking_content_block_start(index: usize) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {
                "type": "thinking",
                "thinking": ""
            }
        })
        .to_string(),
    }
}

fn anthropic_thinking_delta(index: usize, thinking_text: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {
                "type": "thinking_delta",
                "thinking": thinking_text
            }
        })
        .to_string(),
    }
}

fn anthropic_signature_delta(index: usize, signature: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {
                "type": "signature_delta",
                "signature": signature
            }
        })
        .to_string(),
    }
}

fn anthropic_text_delta(index: usize, text: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {
                "type": "text_delta",
                "text": text
            }
        })
        .to_string(),
    }
}

fn anthropic_tool_use_content_block_start(index: usize, id: &str, name: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {
                "type": "tool_use",
                "id": id,
                "name": name
            }
        })
        .to_string(),
    }
}

fn anthropic_input_json_delta(index: usize, partial_json: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {
                "type": "input_json_delta",
                "partial_json": partial_json
            }
        })
        .to_string(),
    }
}

fn anthropic_ping() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({"type": "ping"}).to_string(),
    }
}

fn anthropic_content_block_stop(index: usize) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "content_block_stop",
            "index": index
        })
        .to_string(),
    }
}

fn anthropic_message_delta(usage: &UsageResponse, stop_reason: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": null
            },
            "usage": {
                "output_tokens": usage.completion_tokens.unwrap_or(0),
                "cache_read_input_tokens": usage.cache_hit_tokens.unwrap_or(0)
            }
        })
        .to_string(),
    }
}

fn anthropic_message_stop() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({"type": "message_stop"}).to_string(),
    }
}

// ---------------------------------------------------------------------------
// Public API: SSE generation
// ---------------------------------------------------------------------------

/// Generate an OpenAI-compatible SSE event sequence.
///
/// # Sequence
///
/// 1. `delta.role = "assistant"` — role chunk
/// 2. For each reasoning block: `reasoning_content` delta chunks
/// 3. For each text block: `content` delta chunks
/// 4. For each tool_call block: tool_call start + delta chunks
///    (arguments split by `segment_granularity` into multiple deltas)
/// 5. Finish chunk with `finish_reason = "stop"` or `"tool_calls"`
/// 6. `[DONE]` sentinel
///
/// When `include_usage` is true, the finish chunk carries token usage.
pub fn generate_openai_sse(
    blocks: &[ResponseBlock],
    model: &str,
    usage: &UsageResponse,
    include_usage: bool,
    segment_granularity: usize,
) -> Vec<SseEvent> {
    let mut events = Vec::with_capacity(blocks.len() * 2 + 3);
    events.push(openai_role_chunk(model));

    let has_tool_call = blocks.iter().any(|b| b.block_type == "tool_call");

    for block in blocks {
        match block.block_type.as_str() {
            "reasoning" => {
                if let Some(ref reasoning) = block.reasoning {
                    events.push(openai_reasoning_delta(model, reasoning));
                }
                if let Some(ref content) = block.content {
                    events.push(openai_content_delta(model, content));
                }
            }
            "text" => {
                if let Some(ref content) = block.content {
                    events.push(openai_content_delta(model, content));
                }
            }
            "tool_call" => {
                let call_id = format!("call_{}_{}", model, events.len());
                if let Some(ref name) = block.tool_name {
                    events.push(openai_tool_call_start(model, &call_id, name));
                }
                if let Some(ref args) = block.tool_arguments {
                    if !args.is_empty() {
                        let segments = split_segments(args, segment_granularity);
                        for seg in &segments {
                            events.push(openai_tool_call_delta(model, &call_id, seg));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let finish_reason = if has_tool_call { "tool_calls" } else { "stop" };
    events.push(openai_finish_chunk(
        model,
        usage,
        include_usage,
        finish_reason,
    ));
    events.push(openai_done());
    events
}

/// Generate an Anthropic-compatible SSE event sequence.
///
/// # Sequence
///
/// 1. `message_start` — model name + initial input usage
/// 2. For each content block:
///    - `content_block_start` (type varies by block)
///    - First block gets a `ping` event after start
///    - Delta events + `content_block_stop`
/// 3. `message_delta` — stop_reason + final output usage
/// 4. `message_stop`
pub fn generate_anthropic_sse(
    blocks: &[ResponseBlock],
    model: &str,
    usage: &UsageResponse,
    segment_granularity: usize,
) -> Vec<SseEvent> {
    let has_tool_call = blocks.iter().any(|b| b.block_type == "tool_call");
    let stop_reason = if has_tool_call {
        "tool_use"
    } else {
        "end_turn"
    };

    let mut events = Vec::with_capacity(blocks.len() * 3 + 4);
    events.push(anthropic_message_start(model, usage));

    for (idx, block) in blocks.iter().enumerate() {
        match block.block_type.as_str() {
            "reasoning" => {
                events.push(anthropic_thinking_content_block_start(idx));
                if idx == 0 {
                    events.push(anthropic_ping());
                }
                if let Some(ref reasoning) = block.reasoning {
                    events.push(anthropic_thinking_delta(idx, reasoning));
                }
                if let Some(ref sig) = block.signature {
                    events.push(anthropic_signature_delta(idx, sig));
                }
                events.push(anthropic_content_block_stop(idx));
            }
            "text" => {
                events.push(anthropic_text_content_block_start(idx));
                if idx == 0 {
                    events.push(anthropic_ping());
                }
                if let Some(ref content) = block.content {
                    events.push(anthropic_text_delta(idx, content));
                }
                events.push(anthropic_content_block_stop(idx));
            }
            "tool_call" => {
                let tool_id = format!("toolu_{}_{}", model, idx);
                events.push(anthropic_tool_use_content_block_start(
                    idx,
                    &tool_id,
                    block.tool_name.as_deref().unwrap_or("unknown"),
                ));
                if idx == 0 {
                    events.push(anthropic_ping());
                }
                if let Some(ref args) = block.tool_arguments {
                    if !args.is_empty() {
                        let segments = split_segments(args, segment_granularity);
                        for seg in &segments {
                            events.push(anthropic_input_json_delta(idx, seg));
                        }
                    }
                }
                events.push(anthropic_content_block_stop(idx));
            }
            _ => {}
        }
    }

    events.push(anthropic_message_delta(usage, stop_reason));
    events.push(anthropic_message_stop());
    events
}

// ---------------------------------------------------------------------------
// Common SSE stream types for Axum handlers
// ---------------------------------------------------------------------------

/// Default segment granularity for streaming content splitting.
pub const DEFAULT_SEGMENT_GRANULARITY: usize = 20;

/// Wrapper that yields SSE events from a `Vec`, implementing `futures::Stream`.
///
/// Used by both OpenAI and Anthropic endpoint handlers to convert the
/// `Vec<SseEvent>` from the delivery layer into an Axum SSE response stream.
pub struct SseEventStream {
    inner: std::vec::IntoIter<SseEvent>,
}

impl SseEventStream {
    /// Create a new stream from a vector of SSE events.
    pub fn new(events: Vec<SseEvent>) -> Self {
        Self {
            inner: events.into_iter(),
        }
    }
}

impl futures_core::Stream for SseEventStream {
    type Item = Result<axum::response::sse::Event, std::convert::Infallible>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.inner.next() {
            Some(e) => std::task::Poll::Ready(Some(Ok(to_axum_event(e)))),
            None => std::task::Poll::Ready(None),
        }
    }
}

/// Convert a delivery `SseEvent` into an Axum `Event`.
///
/// Maps the event type and data fields into the SSE wire format.
pub fn to_axum_event(e: SseEvent) -> axum::response::sse::Event {
    let mut event = axum::response::sse::Event::default();
    if !e.event_type.is_empty() {
        event = event.event(e.event_type);
    }
    event.data(e.data)
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    // ------------------------------------------------------------------
    // split_segments
    // ------------------------------------------------------------------

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

    // ------------------------------------------------------------------
    // OpenAI SSE
    // ------------------------------------------------------------------

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

    // ------------------------------------------------------------------
    // Anthropic SSE
    // ------------------------------------------------------------------

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

    // ------------------------------------------------------------------
    // SseEvent structure
    // ------------------------------------------------------------------

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
}
