//! SSE event generation and injection logic for `FakeProvider` delivery layer.
//!
//! Generates protocol-compliant SSE event sequences for both OpenAI and
//! Anthropic protocols. Content is split into segments based on the
//! configured `segment_granularity`.
//!
//! Also provides delay injection and error injection helpers consumed by
//! [`FakeProvider::send`] and [`FakeProvider::send_streaming`].

use super::fake_scenario::DeliveryConfig;
use crate::types::{RawContentBlock, RawUsage};

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

/// OpenAI reasoning content delta event.
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

/// OpenAI tool call start event (first frame with id, type, function.name).
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

/// OpenAI tool call delta event (incremental arguments).
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

/// OpenAI finish chunk event with configurable finish_reason and optional usage.
fn openai_finish_chunk(
    model: &str,
    usage: &RawUsage,
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
/// 2. For each Thinking block: reasoning_content delta chunks
/// 3. For each Text block: content delta chunks
/// 4. For each ToolUse block: tool_call start + delta chunks
///    (input is split by `segment_granularity` into multiple deltas)
/// 5. Finish chunk with `finish_reason = "stop"` or `"tool_calls"`
/// 6. `[DONE]` sentinel (only when `finish_reason = "stop"`)
pub(crate) fn generate_openai_sse(
    content_blocks: &[RawContentBlock],
    model: &str,
    usage: &RawUsage,
    include_usage: bool,
    segment_granularity: usize,
) -> Vec<SseEvent> {
    // Estimate capacity: role + blocks * ~2 + finish + done
    let mut events = Vec::with_capacity(content_blocks.len() * 2 + 3);
    events.push(openai_role_chunk(model));
    let has_tool_use = content_blocks
        .iter()
        .any(|b| matches!(b, RawContentBlock::ToolUse { .. }));
    for block in content_blocks {
        match block {
            RawContentBlock::Thinking { thinking, .. } => {
                events.push(openai_reasoning_delta(model, thinking));
            }
            RawContentBlock::Text(text) => {
                events.push(openai_content_delta(model, text));
            }
            RawContentBlock::ToolUse { id, name, input } => {
                events.push(openai_tool_call_start(model, id, name));
                if !input.is_empty() {
                    let segments = split_segments(input, segment_granularity);
                    for seg in &segments {
                        events.push(openai_tool_call_delta(model, id, seg));
                    }
                }
            }
            _ => {} // ToolResult not relevant for SSE generation
        }
    }
    if has_tool_use {
        events.push(openai_finish_chunk(
            model,
            usage,
            include_usage,
            "tool_calls",
        ));
    } else {
        events.push(openai_finish_chunk(model, usage, include_usage, "stop"));
        events.push(openai_done());
    }
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

/// Anthropic text content_block_start event.
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

/// Anthropic thinking content_block_start event.
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

/// Anthropic thinking_delta event.
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

/// Anthropic signature_delta event.
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

/// Anthropic text_delta event.
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

/// Anthropic tool_use content_block_start event.
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

/// Anthropic input_json_delta event.
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

/// Anthropic ping event.
fn anthropic_ping() -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({"type": "ping"}).to_string(),
    }
}

/// Anthropic content_block_stop event.
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

/// Anthropic message_delta event with stop reason and output usage.
fn anthropic_message_delta(usage: &RawUsage, stop_reason: &str) -> SseEvent {
    SseEvent {
        event_type: "message".into(),
        data: serde_json::json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
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
/// # Arguments
///
/// * `content_blocks` — Raw content blocks (Text, Thinking, ToolUse) to stream.
/// * `model` — Model identifier returned in `message_start`.
/// * `usage` — Token usage for `message_start` and `message_delta`.
/// * `segment_granularity` — Maximum characters per segment. When > 0,
///   ToolUse `input` is split into multiple `input_json_delta` events,
///   each containing at most this many characters. A value of 0 disables
///   segmentation.
///
/// **Note:** Text block splitting into multiple `text_delta` events is
/// handled by the caller (`send_scenario_stream`), not within this
/// function.
///
/// # Sequence
///
/// 1. `message_start` — model name + initial input usage
/// 2. For the first content block:
///    - `content_block_start` (type varies by block)
///    - `ping`
///    - Delta events + `content_block_stop`
/// 3. For subsequent content blocks:
///    - `content_block_start` (type varies by block)
///    - Delta events + `content_block_stop`
/// 4. `message_delta` — stop_reason + final output usage
/// 5. `message_stop`
pub(crate) fn generate_anthropic_sse(
    content_blocks: &[RawContentBlock],
    model: &str,
    usage: &RawUsage,
    segment_granularity: usize,
) -> Vec<SseEvent> {
    let has_tool_use = content_blocks
        .iter()
        .any(|b| matches!(b, RawContentBlock::ToolUse { .. }));
    let stop_reason = if has_tool_use { "tool_use" } else { "end_turn" };
    let mut events = Vec::with_capacity(content_blocks.len() * 3 + 4);
    events.push(anthropic_message_start(model, usage));
    for (idx, block) in content_blocks.iter().enumerate() {
        match block {
            RawContentBlock::Thinking {
                thinking,
                signature,
            } => {
                events.push(anthropic_thinking_content_block_start(idx));
                if idx == 0 {
                    events.push(anthropic_ping());
                }
                events.push(anthropic_thinking_delta(idx, thinking));
                if let Some(sig) = signature {
                    events.push(anthropic_signature_delta(idx, sig));
                }
                events.push(anthropic_content_block_stop(idx));
            }
            RawContentBlock::Text(text) => {
                events.push(anthropic_text_content_block_start(idx));
                if idx == 0 {
                    events.push(anthropic_ping());
                }
                events.push(anthropic_text_delta(idx, text));
                events.push(anthropic_content_block_stop(idx));
            }
            RawContentBlock::ToolUse { id, name, input } => {
                events.push(anthropic_tool_use_content_block_start(idx, id, name));
                if idx == 0 {
                    events.push(anthropic_ping());
                }
                if !input.is_empty() {
                    let segments = split_segments(input, segment_granularity);
                    for seg in &segments {
                        events.push(anthropic_input_json_delta(idx, seg));
                    }
                }
                events.push(anthropic_content_block_stop(idx));
            }
            _ => {} // ToolResult not relevant for SSE generation
        }
    }
    events.push(anthropic_message_delta(usage, stop_reason));
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
#[path = "delivery_tests.rs"]
mod tests;
