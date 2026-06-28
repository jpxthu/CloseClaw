//! GLM (Zhipu AI) ChatProtocol implementation.
//!
//! GLM uses a wire format similar to OpenAI Chat Completions but has some
//! notable differences:
//!   - `reasoning_content` field carries chain-of-thought / thinking content
//!   - Error responses use a nested `{error: {code, message}}` structure
//!   - SSE streaming prioritises `content` delta over `reasoning_content` delta

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::types::{
    ContentBlockType, ContentDelta, InternalMessage, InternalRequest, InternalResponse, ProtocolId,
    RawContentBlock, RawUsage, SseStateMachine, StreamEvent,
};

use crate::protocol::{
    ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError, Result,
};

const PATH: &str = "/api/paas/v4/chat/completions";

/// Minimum trimmed length for `reasoning_content` to be treated as a
/// reasoning block. Shorter values are demoted to plain text.
const MIN_REASONING_LENGTH: usize = 2;

/// GLM protocol implementation.
#[derive(Debug, Clone)]
pub struct GlmProtocol {
    id: ProtocolId,
}

impl GlmProtocol {
    pub fn new() -> Self {
        Self {
            id: ProtocolId::new("glm"),
        }
    }
}

impl Default for GlmProtocol {
    fn default() -> Self {
        Self::new()
    }
}

fn build_message(msg: &InternalMessage) -> serde_json::Value {
    serde_json::json!({
        "role": msg.role,
        "content": msg.content,
    })
}

#[async_trait]
impl ChatProtocol for GlmProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }
    fn path(&self) -> &str {
        PATH
    }

    fn build_request(&self, request: &InternalRequest) -> Result<serde_json::Value> {
        // GLM uses the same request format as OpenAI.
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": request.messages.iter().map(build_message).collect::<Vec<_>>(),
            "temperature": request.temperature,
            "stream": request.stream,
        });

        if let Some(max_tokens) = request.max_tokens {
            body.as_object_mut()
                .unwrap()
                .insert("max_tokens".to_string(), serde_json::json!(max_tokens));
        }

        for (key, value) in &request.extra_body {
            body.as_object_mut()
                .unwrap()
                .insert(key.clone(), value.clone());
        }

        Ok(body)
    }

    fn parse_response(&self, body: serde_json::Value) -> Result<InternalResponse> {
        // GLM may return `reasoning_content` alongside `content`.
        // Prefer `content` if present; fall back to `reasoning_content` if not.
        let choices = body
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("message"));

        let text = choices
            .and_then(|msg| msg.get("content"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let reasoning = choices
            .and_then(|msg| msg.get("reasoning_content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let finish_reason = body
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let usage = parse_usage(&body);

        let content_blocks = match (text, reasoning) {
            (Some(t), _) => vec![RawContentBlock::Text(t)],
            // Non-empty content takes precedence over reasoning (unchanged).
            // Short reasoning (e.g. whitespace-only or scattered chars) is
            // demoted to plain text per design doc requirements.
            (None, Some(r)) if r.trim().len() > MIN_REASONING_LENGTH => {
                vec![RawContentBlock::Thinking {
                    thinking: r,
                    signature: None,
                }]
            }
            (None, Some(r)) => vec![RawContentBlock::Text(r)],
            (None, None) => vec![],
        };

        Ok(InternalResponse {
            content_blocks,
            usage,
            finish_reason,
        })
    }

    fn decorate_headers(&self, headers: &mut HeaderMap) -> Result<()> {
        // GLM uses the same Bearer token format as OpenAI.
        let api_key = std::env::var("GLM_API_KEY").unwrap_or_default();
        let bearer = format!("Bearer {api_key}");
        let auth_value = HeaderValue::from_str(&bearer)
            .map_err(|e| ProtocolError::HeaderDecorate(e.to_string()))?;
        headers.insert(AUTHORIZATION, auth_value);

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
            let mut current_block_index: Option<usize> = None;
            let mut current_block_type: Option<ContentBlockType> = None;
            let mut next_block_index: usize = 0;
            let mut reasoning_buffer: String = String::new();

            while let Some(chunk) = stream.next().await {
                let data = chunk.data.trim();

                if data.is_empty() || data == "[DONE]" {
                    break;
                }

                let parsed: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // GLM SSE delta is in choices[0].delta.
                let delta = match parsed
                    .get("choices")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|c| c.get("delta"))
                {
                    Some(d) => d,
                    None => continue,
                };

                let text_delta = delta.get("content").and_then(|v| v.as_str());
                let thinking_delta = delta.get("reasoning_content").and_then(|v| v.as_str());

                // Buffer reasoning_content deltas; emit at content transition
                // or stream end, enabling short-reasoning filtering equivalent
                // to the non-streaming path.
                if text_delta.is_none() {
                    if let Some(rc) = thinking_delta {
                        if !rc.is_empty() {
                            reasoning_buffer.push_str(rc);
                        }
                    }
                } else {
                    // Content delta — flush reasoning buffer first
                    if !reasoning_buffer.is_empty() {
                        let buf = std::mem::take(&mut reasoning_buffer);
                        let trimmed_len = buf.trim().len();
                        if trimmed_len > MIN_REASONING_LENGTH {
                            let idx = next_block_index;
                            next_block_index += 1;
                            yield StreamEvent::BlockStart {
                                index: idx,
                                block_type: ContentBlockType::Thinking,
                            };
                            yield StreamEvent::BlockDelta {
                                index: idx,
                                delta: ContentDelta::Thinking {
                                    thinking: buf,
                                    signature: None,
                                },
                            };
                            yield StreamEvent::BlockEnd {
                                index: idx,
                                block_type: ContentBlockType::Thinking,
                            };
                        } else if !buf.is_empty() {
                            // Short reasoning — demote to Text, prepend to
                            // the content block that follows.
                            let idx = match current_block_index {
                                Some(i) => i,
                                None => {
                                    let idx = next_block_index;
                                    next_block_index += 1;
                                    current_block_index = Some(idx);
                                    current_block_type = Some(
                                        ContentBlockType::Text,
                                    );
                                    yield StreamEvent::BlockStart {
                                        index: idx,
                                        block_type: ContentBlockType::Text,
                                    };
                                    idx
                                }
                            };
                            yield StreamEvent::BlockDelta {
                                index: idx,
                                delta: ContentDelta::Text {
                                    text: buf,
                                },
                            };
                        }
                    }
                    // text_delta is Some — handled below
                }

                if let Some(text) = text_delta {
                    let idx = match current_block_index {
                        Some(i) => i,
                        None => {
                            let idx = next_block_index;
                            next_block_index += 1;
                            current_block_index = Some(idx);
                            current_block_type = Some(ContentBlockType::Text);
                            yield StreamEvent::BlockStart {
                                index: idx,
                                block_type: ContentBlockType::Text,
                            };
                            idx
                        }
                    };
                    yield StreamEvent::BlockDelta {
                        index: idx,
                        delta: ContentDelta::Text {
                            text: text.to_string(),
                        },
                    };
                }

                // tool_calls delta
                if let Some(tool_calls) = delta.get("tool_calls") {
                    let tool_calls = match tool_calls.as_array() {
                        Some(arr) => arr,
                        None => continue,
                    };

                    // Flush reasoning buffer before tool_calls
                    if !reasoning_buffer.is_empty() {
                        let buf = std::mem::take(&mut reasoning_buffer);
                        let trimmed_len = buf.trim().len();
                        if trimmed_len > MIN_REASONING_LENGTH {
                            let idx = next_block_index;
                            next_block_index += 1;
                            yield StreamEvent::BlockStart {
                                index: idx,
                                block_type: ContentBlockType::Thinking,
                            };
                            yield StreamEvent::BlockDelta {
                                index: idx,
                                delta: ContentDelta::Thinking {
                                    thinking: buf,
                                    signature: None,
                                },
                            };
                            yield StreamEvent::BlockEnd {
                                index: idx,
                                block_type: ContentBlockType::Thinking,
                            };
                        }
                    }

                    // End current block when transitioning to tool_calls
                    if let Some(idx) = current_block_index {
                        if current_block_type != Some(ContentBlockType::ToolUse) {
                            let cur_type = current_block_type
                                .unwrap_or(ContentBlockType::Text);
                            yield StreamEvent::BlockEnd {
                                index: idx,
                                block_type: cur_type,
                            };
                            current_block_index = None;
                            current_block_type = None;
                        }
                    }

                    for tc in tool_calls.iter() {
                        if let Some(tc_id) = tc.get("id")
                            .and_then(|v| v.as_str())
                        {
                            let idx = next_block_index;
                            next_block_index += 1;
                            current_block_index = Some(idx);
                            current_block_type = Some(ContentBlockType::ToolUse);
                            yield StreamEvent::BlockStart {
                                index: idx,
                                block_type: ContentBlockType::ToolUse,
                            };
                            yield StreamEvent::BlockDelta {
                                index: idx,
                                delta: ContentDelta::ToolUseId {
                                    id: tc_id.to_string(),
                                },
                            };
                            if let Some(name) = tc.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                                .filter(|n| !n.is_empty())
                            {
                                yield StreamEvent::BlockDelta {
                                    index: idx,
                                    delta: ContentDelta::ToolUseName {
                                        name: name.to_string(),
                                    },
                                };
                            }
                            if let Some(args) = tc.get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                                .filter(|a| !a.is_empty())
                            {
                                yield StreamEvent::BlockDelta {
                                    index: idx,
                                    delta: ContentDelta::ToolUseInputChunk {
                                        input: args.to_string(),
                                    },
                                };
                            }
                        } else if current_block_type
                            == Some(ContentBlockType::ToolUse)
                        {
                            if let Some(args) = tc.get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                                .filter(|a| !a.is_empty())
                            {
                                let idx = current_block_index.unwrap();
                                yield StreamEvent::BlockDelta {
                                    index: idx,
                                    delta: ContentDelta::ToolUseInputChunk {
                                        input: args.to_string(),
                                    },
                                };
                            }
                        }
                    }
                }

                // finish_reason = "tool_calls" ends the tool block
                let is_tool_calls_finish = parsed
                    .get("choices")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|c| c.get("finish_reason"))
                    .and_then(|v| v.as_str())
                    == Some("tool_calls");
                if is_tool_calls_finish {
                    if let Some(idx) = current_block_index {
                        yield StreamEvent::BlockEnd {
                            index: idx,
                            block_type: ContentBlockType::ToolUse,
                        };
                        current_block_index = None;
                        current_block_type = None;
                    }
                    yield StreamEvent::MessageEnd {
                        usage: None,
                        finish_reason: Some("tool_calls".to_string()),
                    };
                    break;
                }
            }

            // Flush any remaining reasoning buffer at stream end
            if !reasoning_buffer.is_empty() {
                let buf = std::mem::take(&mut reasoning_buffer);
                let trimmed_len = buf.trim().len();
                let idx = next_block_index;
                if trimmed_len > MIN_REASONING_LENGTH {
                    yield StreamEvent::BlockStart {
                        index: idx,
                        block_type: ContentBlockType::Thinking,
                    };
                    yield StreamEvent::BlockDelta {
                        index: idx,
                        delta: ContentDelta::Thinking {
                            thinking: buf,
                            signature: None,
                        },
                    };
                    yield StreamEvent::BlockEnd {
                        index: idx,
                        block_type: ContentBlockType::Thinking,
                    };
                } else {
                    // Short reasoning — demote to Text block
                    yield StreamEvent::BlockStart {
                        index: idx,
                        block_type: ContentBlockType::Text,
                    };
                    yield StreamEvent::BlockDelta {
                        index: idx,
                        delta: ContentDelta::Text {
                            text: buf,
                        },
                    };
                    yield StreamEvent::BlockEnd {
                        index: idx,
                        block_type: ContentBlockType::Text,
                    };
                }
            }

            if let Some(idx) = current_block_index {
                if let Some(btype) = current_block_type {
                    yield StreamEvent::BlockEnd {
                        index: idx,
                        block_type: btype,
                    };
                }
            }

            yield StreamEvent::MessageEnd {
                usage: None,
                finish_reason: Some("stop".to_string()),
            };
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn parse_usage(body: &serde_json::Value) -> RawUsage {
    let usage_obj = body.get("usage");

    let prompt_tokens = usage_obj
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let completion_tokens = usage_obj
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let total_tokens = usage_obj
        .and_then(|u| u.get("total_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    RawUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

#[cfg(test)]
mod tests {
    include!("glm_test.rs");
}
