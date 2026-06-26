//! Anthropic ChatProtocol implementation.
//!
//! Implements `parse_response` with full content block parsing and
//! `parse_sse_stream` with complete Anthropic SSE event handling.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};

use crate::llm::types::{
    ContentBlockType, ContentDelta, InternalMessage, InternalRequest, InternalResponse, ProtocolId,
    RawContentBlock, RawUsage, SseStateMachine, StreamEvent,
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
        incoming: IncomingSseStream,
        _machine: SseStateMachine,
    ) -> OutgoingEventStream {
        Box::pin(async_stream::try_stream! {
            let mut stream = incoming;
            let mut stop_reason: Option<String> = None;
            let mut usage: Option<RawUsage> = None;

            while let Some(chunk) = stream.next().await {
                let data = chunk.data.trim();
                if data.is_empty() {
                    continue;
                }

                let parsed: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let event_type = chunk.event_type.as_str();
                match event_type {
                    "message_start" => {
                        let message = parsed.get("message");
                        if let Some(msg_usage) = message
                            .and_then(|m| m.get("usage"))
                        {
                            usage = Some(parse_usage(
                                msg_usage,
                            ));
                        }
                    }
                    "content_block_start" => {
                        let index = parsed
                            .get("index")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        let block_type = match parsed
                            .get("content_block")
                            .and_then(|cb| cb.get("type"))
                            .and_then(|v| v.as_str())
                        {
                            Some("text") => {
                                ContentBlockType::Text
                            }
                            Some("thinking") => {
                                ContentBlockType::Thinking
                            }
                            Some("tool_use") => {
                                ContentBlockType::ToolUse
                            }
                            _ => continue,
                        };
                        yield StreamEvent::BlockStart {
                            index,
                            block_type,
                        };
                        if block_type
                            == ContentBlockType::ToolUse
                        {
                            let cb = parsed
                                .get("content_block")
                                .unwrap();
                            if let Some(id) = cb
                                .get("id")
                                .and_then(|v| v.as_str())
                            {
                                yield StreamEvent::BlockDelta {
                                    index,
                                    delta: ContentDelta::ToolUseId {
                                        id: id.to_string(),
                                    },
                                };
                            }
                            if let Some(name) = cb
                                .get("name")
                                .and_then(|v| v.as_str())
                            {
                                yield StreamEvent::BlockDelta {
                                    index,
                                    delta: ContentDelta::ToolUseName {
                                        name: name.to_string(),
                                    },
                                };
                            }
                        }
                    }
                    "content_block_delta" => {
                        let index = parsed
                            .get("index")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        let delta = parsed.get("delta");
                        match delta
                            .and_then(|d| d.get("type"))
                            .and_then(|v| v.as_str())
                        {
                            Some("text_delta") => {
                                let text = delta
                                    .and_then(|d| d.get("text"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                yield StreamEvent::BlockDelta {
                                    index,
                                    delta: ContentDelta::Text {
                                        text: text.to_string(),
                                    },
                                };
                            }
                            Some("thinking_delta") => {
                                let thinking = delta
                                    .and_then(|d| d.get("thinking"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                yield StreamEvent::BlockDelta {
                                    index,
                                    delta: ContentDelta::Thinking {
                                        thinking: thinking.to_string(),
                                        signature: None,
                                    },
                                };
                            }
                            Some("signature_delta") => {
                                let value = delta
                                    .and_then(|d| d.get("signature"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                yield StreamEvent::BlockDelta {
                                    index,
                                    delta: ContentDelta::Thinking {
                                        thinking: String::new(),
                                        signature: Some(
                                            value.to_string(),
                                        ),
                                    },
                                };
                            }
                            Some("input_json_delta") => {
                                let input = delta
                                    .and_then(|d| d.get("partial_json"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                yield StreamEvent::BlockDelta {
                                    index,
                                    delta: ContentDelta::ToolUseInputChunk {
                                        input: input.to_string(),
                                    },
                                };
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        let index = parsed
                            .get("index")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        yield StreamEvent::BlockEnd {
                            index,
                            block_type: ContentBlockType::Text,
                        };
                    }
                    "message_delta" => {
                        if let Some(sr) = parsed
                            .get("delta")
                            .and_then(|d| d.get("stop_reason"))
                            .and_then(|v| v.as_str())
                        {
                            stop_reason = Some(
                                sr.to_string(),
                            );
                        }
                        if let Some(msg_usage) = parsed
                            .get("usage")
                        {
                            let output_tokens = msg_usage
                                .get("output_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32;
                            usage = Some(RawUsage {
                                prompt_tokens: usage
                                    .as_ref()
                                    .map(|u| u.prompt_tokens)
                                    .unwrap_or(0),
                                completion_tokens: output_tokens,
                                total_tokens: usage
                                    .as_ref()
                                    .and_then(|u| u.total_tokens),
                                cache_read_tokens: usage
                                    .as_ref()
                                    .and_then(|u| u.cache_read_tokens),
                                cache_write_tokens: usage
                                    .as_ref()
                                    .and_then(|u| u.cache_write_tokens),
                            });
                        }
                    }
                    "message_stop" => {
                        yield StreamEvent::MessageEnd {
                            usage: usage.as_ref().map(|u| u.clone().into()),
                            finish_reason: stop_reason
                                .clone(),
                        };
                    }
                    "error" => {
                        let message = parsed
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error");
                        yield StreamEvent::Error {
                            message: message.to_string(),
                        };
                    }
                    "ping" => {}
                    _ => {}
                }
            }
        })
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
