//! Anthropic ChatProtocol implementation.
//!
//! This is a **stub** implementation: SSE streaming is not yet supported
//! (`parse_sse_stream` returns an empty stream). The HTTP request/response
//! path (`build_request` / `parse_response`) is fully implemented.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};

use crate::llm::types::{
    InternalMessage, InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawUsage,
    SseStateMachine,
};
use crate::session::persistence::ReasoningLevel;

use crate::llm::protocol::{
    ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError, Result,
};

const PATH: &str = "/v1/messages";

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

/// Build an Anthropic `/v1/messages` request body.
///
/// Anthropic request format:
/// ```json
/// {
///   "model": "claude-3-5-sonnet-20241022",
///   "messages": [{"role": "user", "content": "..."}],
///   "max_tokens": 1024,
///   "system": "optional system prompt"
/// }
/// ```
fn build_message(msg: &InternalMessage) -> serde_json::Value {
    serde_json::json!({
        "role": msg.role,
        "content": msg.content,
    })
}

/// Mark the last message with `cache_control` for Anthropic prefix caching.
///
/// The API allows mixing string and content-blocks content formats within
/// the messages array, so only the final message is converted.
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

/// Build the Anthropic `system` array from system blocks.
///
/// Each block with `cache=true` gets a `cache_control` marker.
fn build_system_array(blocks: &[crate::llm::types::SystemBlock]) -> Vec<serde_json::Value> {
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

#[async_trait]
impl ChatProtocol for AnthropicProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }
    fn path(&self) -> &str {
        PATH
    }

    fn build_request(&self, request: &InternalRequest) -> Result<serde_json::Value> {
        // Anthropic does not support reasoning level parameters.
        if request.reasoning_level != ReasoningLevel::High {
            tracing::warn!(
                reasoning_level = %request.reasoning_level,
                "Anthropic protocol does not support reasoning_level parameter, ignoring"
            );
        }

        let mut messages: Vec<serde_json::Value> =
            request.messages.iter().map(build_message).collect();

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

        Ok(body)
    }

    fn parse_response(&self, body: serde_json::Value) -> Result<InternalResponse> {
        // Anthropic response: { "content": [{"type": "text", "text": "..."}] }
        let content_blocks: Vec<RawContentBlock> = body
            .get("content")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let ty = item.get("type").and_then(|v| v.as_str());
                        match ty {
                            Some("text") => item
                                .get("text")
                                .and_then(|v| v.as_str())
                                .map(|s| RawContentBlock::Text(s.to_string())),
                            Some("thinking") => {
                                let thinking =
                                    item.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
                                let signature = item
                                    .get("signature")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                Some(RawContentBlock::Thinking {
                                    thinking: thinking.to_string(),
                                    signature,
                                })
                            }
                            Some("tool_use") => {
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
                            Some("tool_result") => {
                                let tool_call_id = item
                                    .get("tool_use_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let content = item
                                    .get("content")
                                    .map(|v| {
                                        if let Some(s) = v.as_str() {
                                            s.to_string()
                                        } else if let Some(arr) = v.as_array() {
                                            arr.iter()
                                                .filter_map(|block| {
                                                    if block.get("type").and_then(|t| t.as_str())
                                                        == Some("text")
                                                    {
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
                                    .unwrap_or_default();
                                Some(RawContentBlock::ToolResult {
                                    tool_call_id,
                                    content,
                                })
                            }
                            _ => None,
                        }
                    })
                    .collect()
            })
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
        _incoming: IncomingSseStream,
        _machine: SseStateMachine,
    ) -> OutgoingEventStream {
        // Stub: streaming not yet implemented — return an empty stream.
        Box::pin(futures::stream::empty())
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

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
