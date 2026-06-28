//! Anthropic ChatProtocol implementation.
//!
//! Implements `parse_response` with full content block parsing and
//! `parse_sse_stream` with complete Anthropic SSE event handling.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use std::collections::HashMap;

use crate::types::{
    ContentBlockType, ContentDelta, InternalMessage, InternalRequest, InternalResponse, ProtocolId,
    RawContentBlock, RawUsage, SseStateMachine, StreamEvent, ToolDefinition,
};
use closeclaw_session::persistence::ReasoningLevel;

use crate::protocol::{
    ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError, Result,
};

const PATH: &str = "/v1/messages";

// ─── AnthropicProtocol ───────────────────────────────────────────────────────

/// Anthropic protocol implementation.
#[derive(Debug, Clone)]
pub struct AnthropicProtocol {
    id: ProtocolId,
}

impl AnthropicProtocol {
    pub fn new() -> Self {
        Self {
            id: ProtocolId::new("anthropic"),
        }
    }
}

impl Default for AnthropicProtocol {
    fn default() -> Self {
        Self::new()
    }
}

// ─── ChatProtocol trait implementation ───────────────────────────────────────

#[async_trait]
impl ChatProtocol for AnthropicProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }

    fn path(&self) -> &str {
        PATH
    }

    fn build_request(&self, request: &InternalRequest) -> Result<serde_json::Value> {
        build_request_body(request)
    }

    fn parse_response(&self, body: serde_json::Value) -> Result<InternalResponse> {
        parse_response_body(body)
    }

    fn decorate_headers(&self, headers: &mut HeaderMap) -> Result<()> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        let key_value = HeaderValue::from_str(&api_key)
            .map_err(|e| ProtocolError::HeaderDecorate(e.to_string()))?;
        headers.insert("x-api-key", key_value);

        let version_value = HeaderValue::from_static("2023-06-01");
        headers.insert("anthropic-version", version_value);

        let ct_value = HeaderValue::from_static("application/json");
        headers.insert(CONTENT_TYPE, ct_value);

        Ok(())
    }

    fn create_sse_machine(&self) -> SseStateMachine {
        SseStateMachine::new()
    }

    async fn parse_sse_stream(
        &self,
        incoming: IncomingSseStream,
        _machine: SseStateMachine,
    ) -> OutgoingEventStream {
        parse_sse(incoming)
    }
}

// ─── build_request helpers ───────────────────────────────────────────────────

/// Build an Anthropic `/v1/messages` request body.
fn build_request_body(request: &InternalRequest) -> Result<serde_json::Value> {
    if request.reasoning_level != ReasoningLevel::High {
        tracing::warn!(
            reasoning_level = %request.reasoning_level,
            "Anthropic protocol does not support reasoning_level parameter, ignoring"
        );
    }

    let mut messages: Vec<serde_json::Value> = request.messages.iter().map(build_message).collect();

    mark_last_message_cache_control(&mut messages);

    let mut body = serde_json::json!({
        "model": request.model,
        "messages": messages,
    });

    if let Some(max_tokens) = request.max_tokens {
        body.as_object_mut()
            .unwrap()
            .insert("max_tokens".to_string(), serde_json::json!(max_tokens));
    }

    // Only include temperature if it differs from the default (0.0).
    if request.temperature != 0.0 {
        body.as_object_mut().unwrap().insert(
            "temperature".to_string(),
            serde_json::json!(request.temperature),
        );
    }

    if let Some(ref blocks) = request.system_blocks {
        if !blocks.is_empty() {
            let system_array = build_system_array(blocks);
            body.as_object_mut()
                .unwrap()
                .insert("system".to_string(), serde_json::json!(system_array));
        }
    }

    for (key, value) in &request.extra_body {
        body.as_object_mut()
            .unwrap()
            .insert(key.clone(), value.clone());
    }

    if let Some(ref tools) = request.tools {
        if !tools.is_empty() {
            body.as_object_mut().unwrap().insert(
                "tools".to_string(),
                serde_json::json!(build_tools_array(tools)),
            );
        }
    }

    Ok(body)
}

/// Build a single message JSON for Anthropic.
///
/// Tool result messages use role "user" with structured content blocks,
/// matching Anthropic's native tool_result format.
fn build_message(msg: &InternalMessage) -> serde_json::Value {
    if let Some(ref tool_call_id) = msg.tool_call_id {
        serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": msg.content,
            }],
        })
    } else {
        serde_json::json!({
            "role": msg.role,
            "content": msg.content,
        })
    }
}

/// Mark the last message with `cache_control` for Anthropic prefix caching.
fn mark_last_message_cache_control(messages: &mut [serde_json::Value]) {
    if let Some(last_msg) = messages.last_mut() {
        if let Some(content) = last_msg.get("content").and_then(|c| c.as_str()) {
            let text = content.to_string();
            last_msg.as_object_mut().unwrap().insert(
                "content".to_string(),
                serde_json::json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": { "type": "ephemeral" }
                }]),
            );
        }
    }
}

/// Build an Anthropic tools array from tool definitions.
///
/// Each tool includes `name`, `description`, and `input_schema`.
/// Tools with `cache: true` also include `cache_control`.
fn build_tools_array(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|tool| {
            let mut obj = serde_json::json!({
                "name": tool.name,
                "description": tool.description,
            });
            if let Some(ref schema) = tool.input_schema {
                obj.as_object_mut()
                    .unwrap()
                    .insert("input_schema".to_string(), schema.clone());
            }
            if tool.cache {
                obj.as_object_mut().unwrap().insert(
                    "cache_control".to_string(),
                    serde_json::json!({ "type": "ephemeral" }),
                );
            }
            obj
        })
        .collect()
}

/// Build the Anthropic `system` array from system blocks.
fn build_system_array(blocks: &[crate::types::SystemBlock]) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .map(|b| {
            let mut obj = serde_json::json!({
                "type": "text",
                "text": b.text,
            });
            if b.cache {
                obj.as_object_mut().unwrap().insert(
                    "cache_control".to_string(),
                    serde_json::json!({ "type": "ephemeral" }),
                );
            }
            obj
        })
        .collect()
}

// ─── parse_response helpers ──────────────────────────────────────────────────

/// Parse Anthropic response body into `InternalResponse`.
fn parse_response_body(body: serde_json::Value) -> Result<InternalResponse> {
    let content_blocks: Vec<RawContentBlock> = body
        .get("content")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_content_block).collect())
        .unwrap_or_default();

    let usage = parse_usage(&body);
    let finish_reason = body
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(InternalResponse {
        content_blocks,
        usage,
        finish_reason,
    })
}

/// Parse a single content block from the Anthropic response.
fn parse_content_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    let ty = item.get("type").and_then(|v| v.as_str());
    match ty {
        Some("text") => parse_text_block(item),
        Some("thinking") => parse_thinking_block(item),
        Some("tool_use") => parse_tool_use_block(item),
        Some("tool_result") => parse_tool_result_block(item),
        _ => None,
    }
}

/// Parse a text content block.
fn parse_text_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    item.get("text")
        .and_then(|v| v.as_str())
        .map(|s| RawContentBlock::Text(s.to_string()))
}

/// Parse a thinking content block.
fn parse_thinking_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    let thinking = item.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
    let signature = item
        .get("signature")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(RawContentBlock::Thinking {
        thinking: thinking.to_string(),
        signature,
    })
}

/// Parse a tool_use content block from Anthropic response.
fn parse_tool_use_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = item
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let input = item
        .get("input")
        .map(|v| {
            if v.is_object() && v.as_object().unwrap().is_empty() {
                "{}".to_string()
            } else {
                v.to_string()
            }
        })
        .unwrap_or_else(|| "{}".to_string());
    Some(RawContentBlock::ToolUse { id, name, input })
}

/// Parse a tool_result content block from Anthropic response.
fn parse_tool_result_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    let tool_call_id = item
        .get("tool_use_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let content = extract_tool_result_content(item);
    Some(RawContentBlock::ToolResult {
        tool_call_id,
        content,
    })
}

/// Extract content from a tool_result block.
///
/// The `content` field can be a string or an array of content blocks.
fn extract_tool_result_content(item: &serde_json::Value) -> String {
    item.get("content")
        .map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else if let Some(arr) = v.as_array() {
                arr.iter()
                    .filter_map(|block| {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            block
                                .get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("")
            } else {
                String::new()
            }
        })
        .unwrap_or_default()
}

/// Parse usage information from a JSON value.
fn parse_usage(body: &serde_json::Value) -> RawUsage {
    let usage_obj = body.get("usage");

    let input_tokens = usage_obj
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let output_tokens = usage_obj
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let total_tokens = usage_obj
        .and_then(|u| u.get("total_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let cache_read_tokens = usage_obj
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let cache_write_tokens = usage_obj
        .and_then(|u| u.get("cache_creation_input_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    RawUsage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        cache_read_tokens,
        cache_write_tokens,
    }
}

// ─── parse_sse_stream helpers ────────────────────────────────────────────────

/// Parse an Anthropic SSE stream into unified `StreamEvent`s.
fn parse_sse(incoming: IncomingSseStream) -> OutgoingEventStream {
    Box::pin(async_stream::try_stream! {
        let mut stream = incoming;
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<RawUsage> = None;
        let mut block_type_map: HashMap<usize, ContentBlockType> = HashMap::new();

        while let Some(chunk) = stream.next().await {
            let data = chunk.data.trim();
            if data.is_empty() {
                continue;
            }
            let parsed: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for event in dispatch_sse_event(&chunk.event_type, &parsed, &mut block_type_map, &mut usage, &mut stop_reason) {
                yield event;
            }
        }
    })
}

/// Dispatch a single SSE event into one or more `StreamEvent`s.
fn dispatch_sse_event(
    event_type: &str,
    parsed: &serde_json::Value,
    block_type_map: &mut HashMap<usize, ContentBlockType>,
    usage: &mut Option<RawUsage>,
    stop_reason: &mut Option<String>,
) -> Vec<StreamEvent> {
    match event_type {
        "message_start" => {
            *usage = parse_message_start_usage(parsed).or_else(|| usage.take());
            vec![]
        }
        "content_block_start" => parse_content_block_start(parsed, block_type_map),
        "content_block_delta" => parse_content_block_delta(parsed).into_iter().collect(),
        "content_block_stop" => parse_content_block_stop(parsed, block_type_map)
            .into_iter()
            .collect(),
        "message_delta" => {
            if let Some(sr) = extract_stop_reason(parsed) {
                *stop_reason = Some(sr);
            }
            update_usage_from_message_delta(parsed, usage);
            vec![]
        }
        "message_stop" => {
            vec![StreamEvent::MessageEnd {
                usage: usage.as_ref().map(|u| u.clone().into()),
                finish_reason: stop_reason.clone(),
            }]
        }
        "error" => {
            vec![StreamEvent::Error {
                message: extract_error_message(parsed),
            }]
        }
        _ => vec![],
    }
}

/// Parse initial usage from `message_start` event.
fn parse_message_start_usage(parsed: &serde_json::Value) -> Option<RawUsage> {
    parsed
        .get("message")
        .and_then(|m| m.get("usage"))
        .map(parse_usage)
}

/// Handle `content_block_start` event: record block type and emit events.
fn parse_content_block_start(
    parsed: &serde_json::Value,
    block_type_map: &mut HashMap<usize, ContentBlockType>,
) -> Vec<StreamEvent> {
    let index = parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let block_type = match parsed
        .get("content_block")
        .and_then(|cb| cb.get("type"))
        .and_then(|v| v.as_str())
    {
        Some("text") => ContentBlockType::Text,
        Some("thinking") => ContentBlockType::Thinking,
        Some("tool_use") => ContentBlockType::ToolUse,
        _ => return vec![],
    };

    block_type_map.insert(index, block_type);
    let mut events = vec![StreamEvent::BlockStart { index, block_type }];

    // For tool_use, also yield id and name deltas
    if block_type == ContentBlockType::ToolUse {
        let cb = parsed.get("content_block").unwrap();
        if let Some(id) = cb.get("id").and_then(|v| v.as_str()) {
            events.push(StreamEvent::BlockDelta {
                index,
                delta: ContentDelta::ToolUseId { id: id.to_string() },
            });
        }
        if let Some(name) = cb.get("name").and_then(|v| v.as_str()) {
            events.push(StreamEvent::BlockDelta {
                index,
                delta: ContentDelta::ToolUseName {
                    name: name.to_string(),
                },
            });
        }
    }

    events
}

/// Handle `content_block_delta` event.
fn parse_content_block_delta(parsed: &serde_json::Value) -> Option<StreamEvent> {
    let index = parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let delta = parsed.get("delta")?;

    match delta.get("type").and_then(|v| v.as_str()) {
        Some("text_delta") => {
            let text = delta.get("text").and_then(|v| v.as_str()).unwrap_or("");
            Some(StreamEvent::BlockDelta {
                index,
                delta: ContentDelta::Text {
                    text: text.to_string(),
                },
            })
        }
        Some("thinking_delta") => {
            let thinking = delta.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
            Some(StreamEvent::BlockDelta {
                index,
                delta: ContentDelta::Thinking {
                    thinking: thinking.to_string(),
                    signature: None,
                },
            })
        }
        Some("signature_delta") => {
            let value = delta
                .get("signature")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(StreamEvent::BlockDelta {
                index,
                delta: ContentDelta::Thinking {
                    thinking: String::new(),
                    signature: Some(value.to_string()),
                },
            })
        }
        Some("input_json_delta") => {
            let input = delta
                .get("partial_json")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(StreamEvent::BlockDelta {
                index,
                delta: ContentDelta::ToolUseInputChunk {
                    input: input.to_string(),
                },
            })
        }
        _ => None,
    }
}

/// Handle `content_block_stop` event.
fn parse_content_block_stop(
    parsed: &serde_json::Value,
    block_type_map: &mut HashMap<usize, ContentBlockType>,
) -> Option<StreamEvent> {
    let index = parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let block_type = block_type_map
        .remove(&index)
        .unwrap_or(ContentBlockType::Text);
    Some(StreamEvent::BlockEnd { index, block_type })
}

/// Extract stop_reason from `message_delta` event.
fn extract_stop_reason(parsed: &serde_json::Value) -> Option<String> {
    parsed
        .get("delta")
        .and_then(|d| d.get("stop_reason"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Update usage from `message_delta` event.
fn update_usage_from_message_delta(parsed: &serde_json::Value, usage: &mut Option<RawUsage>) {
    let msg_usage = match parsed.get("usage") {
        Some(u) => u,
        None => return,
    };

    let output_tokens = msg_usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    *usage = Some(RawUsage {
        prompt_tokens: usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
        completion_tokens: output_tokens,
        total_tokens: usage.as_ref().and_then(|u| u.total_tokens),
        cache_read_tokens: usage.as_ref().and_then(|u| u.cache_read_tokens),
        cache_write_tokens: usage.as_ref().and_then(|u| u.cache_write_tokens),
    });
}

/// Extract error message from an `error` event.
fn extract_error_message(parsed: &serde_json::Value) -> String {
    parsed
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown error")
        .to_string()
}
