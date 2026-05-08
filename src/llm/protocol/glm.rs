//! GLM (Zhipu AI) ChatProtocol implementation.
//!
//! GLM uses a wire format similar to OpenAI Chat Completions but has some
//! notable differences:
//!   - `reasoning_content` field carries chain-of-thought / thinking content
//!   - Error responses use a nested `{error: {code, message}}` structure
//!   - SSE streaming prioritises `reasoning_content` delta over `content` delta

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::llm::types::{
    ContentBlockType, ContentDelta, InternalMessage, InternalRequest, InternalResponse, ProtocolId,
    RawContentBlock, RawUsage, SseStateMachine, StreamEvent,
};

use crate::llm::protocol::{
    ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError, Result,
};

const PATH: &str = "/api/paas/v4/chat/completions";

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
            (_, Some(r)) => vec![RawContentBlock::Thinking(r)],
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

                // GLM may emit `reasoning_content` delta or `content` delta.
                // Prefer `reasoning_content` first, then fall back to `content`.
                let (text_delta, thinking_delta) = (
                    delta.get("content").and_then(|v| v.as_str()),
                    delta.get("reasoning_content").and_then(|v| v.as_str()),
                );

                let content_to_yield = thinking_delta.or(text_delta);

                if let Some(text) = content_to_yield {
                    let (idx, btype) = match current_block_index {
                        Some(i) => (i, current_block_type.unwrap()),
                        None => {
                            // First delta — emit BlockStart.
                            let btype = if thinking_delta.is_some() {
                                ContentBlockType::Thinking
                            } else {
                                ContentBlockType::Text
                            };
                            let idx = 0;
                            current_block_index = Some(idx);
                            current_block_type = Some(btype);
                            yield StreamEvent::BlockStart { index: idx, block_type: btype };
                            (idx, btype)
                        }
                    };

                    let delta = match btype {
                        ContentBlockType::Thinking => {
                            ContentDelta::Thinking { thinking: text.to_string() }
                        }
                        _ => ContentDelta::Text { text: text.to_string() },
                    };

                    yield StreamEvent::BlockDelta { index: idx, delta };
                }
            }

            if let Some(idx) = current_block_index {
                if let Some(btype) = current_block_type {
                    yield StreamEvent::BlockEnd { index: idx, block_type: btype };
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
    }
}

#[cfg(test)]
mod tests {
    include!("glm_test.rs");
}
