//! OpenAI-compatible ChatProtocol implementation.
//!
//! Supports OpenAI, MiniMax, VolcEngine, and DeepSeek - all of which share
//! the same OpenAI Chat Completions wire format.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::protocol::{
    ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError, Result,
};
use crate::types::{
    ContentBlockType, ContentDelta, InternalMessage, InternalRequest, InternalResponse, ProtocolId,
    RawContentBlock, RawUsage, SseStateMachine, StreamEvent,
};

const PATH: &str = "/v1/chat/completions";

/// OpenAI-compatible protocol implementation.
#[derive(Debug, Clone)]
pub struct OpenAiProtocol {
    id: ProtocolId,
}

impl OpenAiProtocol {
    pub fn new() -> Self {
        Self {
            id: ProtocolId::new("openai"),
        }
    }
}

impl Default for OpenAiProtocol {
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
impl ChatProtocol for OpenAiProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }
    fn path(&self) -> &str {
        PATH
    }

    fn build_request(&self, request: &InternalRequest) -> Result<serde_json::Value> {
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
        let message = body
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("message"));

        let content = message
            .and_then(|msg| msg.get("content"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let reasoning_content = message
            .and_then(|msg| msg.get("reasoning_content"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let finish_reason = body
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let usage = parse_usage(&body);

        // 按文档规则构建 content_blocks：
        // content 非空 + reasoning_content 非空 → Thinking + Text
        // content 非空 + reasoning_content 空 → Text
        // content 空 + reasoning_content 非空 → Text（reasoning_content 作为 Text 内容）
        // 两者都空 → 空 Text
        let mut content_blocks = Vec::new();
        if let Some(ref text) = content {
            // content 非空：两者独立产出各自块（Thinking 在前）
            if let Some(thinking) = reasoning_content {
                content_blocks.push(RawContentBlock::Thinking {
                    thinking,
                    signature: None,
                });
            }
            content_blocks.push(RawContentBlock::Text(text.clone()));
        } else if let Some(thinking) = reasoning_content {
            // content 空 + reasoning_content 非空 → reasoning_content 作为 Text 块
            content_blocks.push(RawContentBlock::Text(thinking));
        } else {
            // 两者都空 → 空 Text 块
            content_blocks.push(RawContentBlock::Text(String::new()));
        };

        // Parse tool_calls from message (doc: choices[].message.tool_calls[] → ToolUse)
        if let Some(tool_calls) = message
            .and_then(|msg| msg.get("tool_calls"))
            .and_then(|v| v.as_array())
        {
            for tc in tool_calls {
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let input = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                content_blocks.push(RawContentBlock::ToolUse { id, name, input });
            }
        }

        Ok(InternalResponse {
            content_blocks,
            usage,
            finish_reason,
        })
    }

    fn decorate_headers(&self, headers: &mut HeaderMap) -> Result<()> {
        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
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
            let mut block_index: Option<usize> = None;
            let mut next_block_index: usize = 0;
            let mut active_block_type: Option<ContentBlockType> = None;

            while let Some(chunk) = stream.next().await {
                let data = chunk.data.trim();
                if data.is_empty() || data == "[DONE]" {
                    break;
                }

                let parsed: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let choices = match parsed.get("choices").and_then(|v| v.as_array()) {
                    Some(arr) if !arr.is_empty() => arr,
                    _ => continue,
                };
                let choice = &choices[0];
                let delta = match choice.get("delta") {
                    Some(d) => d,
                    None => continue,
                };
                let finish_reason = choice.get("finish_reason").and_then(|v| v.as_str());

                // End current block when transitioning to tool_calls
                if delta.get("tool_calls").and_then(|v| v.as_array()).is_some() {
                    if let Some(idx) = block_index {
                        // Only end non-tool blocks (text/thinking → tool_calls transition)
                        if active_block_type != Some(ContentBlockType::ToolUse) {
                            let cur_type = active_block_type.unwrap_or(ContentBlockType::Text);
                            yield StreamEvent::BlockEnd { index: idx, block_type: cur_type };
                            block_index = None;
                            active_block_type = None;
                        }
                    }
                }

                // Content delta
                // Document rule: content is prioritized over reasoning_content.
                // If a Text block is already active, append to it.
                // If a Thinking block is active and non-empty content arrives,
                // end Thinking and start Text (content wins).
                if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                    if text.is_empty() {
                        continue;
                    }
                    // If Thinking block is active, content takes priority: end it
                    if active_block_type == Some(ContentBlockType::Thinking) {
                        if let Some(prev_idx) = block_index {
                            yield StreamEvent::BlockEnd {
                                index: prev_idx,
                                block_type: ContentBlockType::Thinking,
                            };
                        }
                        block_index = None;
                        active_block_type = None;
                    }
                    let idx = match block_index {
                        Some(i) => i,
                        None => {
                            let idx = next_block_index;
                            next_block_index += 1;
                            block_index = Some(idx);
                            active_block_type = Some(ContentBlockType::Text);
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

                // reasoning_content delta
                // If a Text block is already active, content is non-empty so
                // reasoning_content is ignored (content priority).
                if let Some(thinking) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                    if thinking.is_empty() {
                        continue;
                    }
                    // If Text block is active, ignore reasoning_content
                    if active_block_type == Some(ContentBlockType::Text) {
                        continue;
                    }
                    let idx = match block_index {
                        Some(i) if active_block_type == Some(ContentBlockType::Thinking) => i,
                        _ => {
                            // End current block if any (e.g., tool_calls → thinking)
                            if let Some(prev_idx) = block_index {
                                let prev_type = active_block_type.unwrap_or(
                                    ContentBlockType::Text,
                                );
                                yield StreamEvent::BlockEnd {
                                    index: prev_idx,
                                    block_type: prev_type,
                                };
                            }
                            let idx = next_block_index;
                            next_block_index += 1;
                            block_index = Some(idx);
                            active_block_type = Some(ContentBlockType::Thinking);
                            yield StreamEvent::BlockStart {
                                index: idx,
                                block_type: ContentBlockType::Thinking,
                            };
                            idx
                        }
                    };
                    yield StreamEvent::BlockDelta {
                        index: idx,
                        delta: ContentDelta::Thinking {
                            thinking: thinking.to_string(),
                            signature: None,
                        },
                    };
                }

                // tool_calls delta
                if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls.iter() {
                        if let Some(tc_id) = tc.get("id").and_then(|v| v.as_str()) {
                            // Start new tool block
                            let idx = next_block_index;
                            next_block_index += 1;
                            block_index = Some(idx);
                            active_block_type = Some(ContentBlockType::ToolUse);
                            yield StreamEvent::BlockStart { index: idx, block_type: ContentBlockType::ToolUse };
                            yield StreamEvent::BlockDelta { index: idx, delta: ContentDelta::ToolUseId { id: tc_id.to_string() } };

                            if let Some(name) = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).filter(|n| !n.is_empty()) {
                                yield StreamEvent::BlockDelta { index: idx, delta: ContentDelta::ToolUseName { name: name.to_string() } };
                            }
                            if let Some(args) = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).filter(|a| !a.is_empty()) {
                                yield StreamEvent::BlockDelta { index: idx, delta: ContentDelta::ToolUseInputChunk { input: args.to_string() } };
                            }
                        } else if active_block_type == Some(ContentBlockType::ToolUse) {
                            // Continuation: arguments chunk
                            if let Some(args) = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).filter(|a| !a.is_empty()) {
                                yield StreamEvent::BlockDelta { index: block_index.unwrap(), delta: ContentDelta::ToolUseInputChunk { input: args.to_string() } };
                            }
                        }
                    }
                }

                // finish_reason = "tool_calls" ends the tool block
                if finish_reason == Some("tool_calls") {
                    if let Some(idx) = block_index {
                        yield StreamEvent::BlockEnd { index: idx, block_type: ContentBlockType::ToolUse };
                        block_index = None;
                        active_block_type = None;
                    }
                    yield StreamEvent::MessageEnd { usage: None, finish_reason: Some("tool_calls".to_string()) };
                    break;
                }
            }

            if let Some(idx) = block_index {
                let cur_type = active_block_type.unwrap_or(ContentBlockType::Text);
                yield StreamEvent::BlockEnd { index: idx, block_type: cur_type };
            }
            yield StreamEvent::MessageEnd { usage: None, finish_reason: Some("stop".to_string()) };
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

    let cache_read_tokens = usage_obj
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    RawUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_tokens,
        cache_write_tokens: None,
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod openai_tests; // extracted to stay under 500-line limit
