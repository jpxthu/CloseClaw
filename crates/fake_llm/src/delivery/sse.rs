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
    let mut usage_json = serde_json::json!({
        "input_tokens": usage.prompt_tokens.unwrap_or(0),
        "output_tokens": 0
    });
    if !usage.cache_fields_missing || usage.cache_hit_tokens.is_some() {
        usage_json["cache_read_input_tokens"] =
            serde_json::json!(usage.cache_hit_tokens.unwrap_or(0));
    }
    if !usage.cache_fields_missing || usage.cache_write_tokens.is_some() {
        usage_json["cache_creation_input_tokens"] =
            serde_json::json!(usage.cache_write_tokens.unwrap_or(0));
    }
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "message",
            "id": msg_id,
            "role": "assistant",
            "content": [],
            "model": model,
            "stop_reason": null,
            "usage": usage_json
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
    let mut usage_json = serde_json::json!({
        "output_tokens": usage.completion_tokens.unwrap_or(0)
    });
    if !usage.cache_fields_missing || usage.cache_hit_tokens.is_some() {
        usage_json["cache_read_input_tokens"] =
            serde_json::json!(usage.cache_hit_tokens.unwrap_or(0));
    }
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": null
            },
            "usage": usage_json
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
