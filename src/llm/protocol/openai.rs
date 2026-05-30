//! OpenAI-compatible ChatProtocol implementation.
//!
//! Supports OpenAI, MiniMax, VolcEngine, and DeepSeek - all of which share
//! the same OpenAI Chat Completions wire format.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::llm::types::{
    ContentBlockType, ContentDelta, InternalMessage, InternalRequest, InternalResponse, ProtocolId,
    RawContentBlock, RawUsage, SseStateMachine, StreamEvent,
};
use crate::session::persistence::ReasoningLevel;

use crate::llm::protocol::{
    ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError, Result,
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

        // Map ReasoningLevel → reasoning_effort for DeepSeek and OpenAI-compatible providers.
        match request.reasoning_level {
            ReasoningLevel::Low => {
                body.as_object_mut()
                    .unwrap()
                    .insert("reasoning_effort".to_string(), serde_json::json!("off"));
            }
            ReasoningLevel::Medium => {
                body.as_object_mut()
                    .unwrap()
                    .insert("reasoning_effort".to_string(), serde_json::json!("base"));
            }
            ReasoningLevel::High => {
                body.as_object_mut()
                    .unwrap()
                    .insert("reasoning_effort".to_string(), serde_json::json!("high"));
            }
            ReasoningLevel::Max => {
                body.as_object_mut().unwrap().insert(
                    "reasoning_effort".to_string(),
                    serde_json::json!("reasoner"),
                );
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
        let choices = body
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|msg| msg.get("content"))
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

        let content_blocks = choices
            .map(|text| vec![RawContentBlock::Text(text)])
            .unwrap_or_default();

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
                if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                    if text.is_empty() {
                        continue;
                    }
                    let idx = match block_index {
                        Some(i) => i,
                        None => {
                            let idx = next_block_index;
                            next_block_index += 1;
                            block_index = Some(idx);
                            active_block_type = Some(ContentBlockType::Text);
                            yield StreamEvent::BlockStart { index: idx, block_type: ContentBlockType::Text };
                            idx
                        }
                    };
                    yield StreamEvent::BlockDelta { index: idx, delta: ContentDelta::Text { text: text.to_string() } };
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

    RawUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod openai_tests; // extracted to stay under 500-line limit
