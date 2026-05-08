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

            while let Some(chunk) = stream.next().await {
                let data = chunk.data.trim();

                if data.is_empty() || data == "[DONE]" {
                    break;
                }

                let parsed: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let delta = match parsed
                    .get("choices")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|c| c.get("delta"))
                {
                    Some(d) => d,
                    None => continue,
                };

                // Handle role at block start.
                if let Some(_role) = delta.get("role").and_then(|v| v.as_str()) {
                    if block_index.is_none() {
                        block_index = Some(0);
                        yield StreamEvent::BlockStart {
                            index: 0,
                            block_type: ContentBlockType::Text,
                        };
                    }
                    // Role-only messages have no content delta.
                    continue;
                }

                // Content delta.
                if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                    let idx = match block_index {
                        Some(i) => i,
                        None => {
                            block_index = Some(0);
                            let idx = 0;
                            yield StreamEvent::BlockStart {
                                index: idx,
                                block_type: ContentBlockType::Text,
                            };
                            idx
                        }
                    };
                    yield StreamEvent::BlockDelta {
                        index: idx,
                        delta: ContentDelta::Text { text: text.to_string() },
                    };
                }
            }

            if let Some(idx) = block_index {
                yield StreamEvent::BlockEnd {
                    index: idx,
                    block_type: ContentBlockType::Text,
                };
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
    use super::*;
    use crate::llm::types::InternalMessage;

    fn make_request() -> InternalRequest {
        InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            temperature: 0.7,
            max_tokens: Some(256),
            stream: false,
            extra_body: Default::default(),
        }
    }

    fn make_request_with_extra() -> InternalRequest {
        InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            temperature: 0.7,
            max_tokens: Some(256),
            stream: false,
            extra_body: serde_json::from_str(r#"{"seed":42,"frequency_penalty":0.5}"#).unwrap(),
        }
    }

    // ── build_request tests ───────────────────────────────────────────────────

    #[test]
    fn test_build_request_basic() {
        let proto = OpenAiProtocol::new();
        let request = make_request();
        let body = proto.build_request(&request).unwrap();

        assert_eq!(body.get("model").unwrap(), "gpt-4");
        assert!(body.get("messages").unwrap().is_array());
        // Note: f32 0.7 serializes to JSON as 0.699999988079071 due to IEEE-754 precision.
        // Use (value - 0.7).abs() < 1e-6 for float comparison to avoid exact bit comparison.
        let temp_val = body.get("temperature").unwrap().as_f64().unwrap();
        assert!(
            (temp_val - 0.7).abs() < 1e-6,
            "temperature = {} expected ~0.7",
            temp_val
        );
        assert_eq!(body.get("max_tokens").unwrap(), &serde_json::json!(256));
        assert_eq!(body.get("stream").unwrap(), &serde_json::json!(false));
    }

    #[test]
    fn test_build_request_extra_body_merge() {
        let proto = OpenAiProtocol::new();
        let request = make_request_with_extra();
        let body = proto.build_request(&request).unwrap();

        assert_eq!(body.get("seed").unwrap(), &serde_json::json!(42));
        assert_eq!(
            body.get("frequency_penalty").unwrap(),
            &serde_json::json!(0.5)
        );
    }

    #[test]
    fn test_build_request_stream_flag() {
        let proto = OpenAiProtocol::new();
        let mut request = make_request();
        request.stream = true;
        let body = proto.build_request(&request).unwrap();
        assert_eq!(body.get("stream").unwrap(), &serde_json::json!(true));
    }

    #[test]
    fn test_build_request_no_max_tokens() {
        let proto = OpenAiProtocol::new();
        let mut request = make_request();
        request.max_tokens = None;
        let body = proto.build_request(&request).unwrap();
        assert!(body.get("max_tokens").is_none());
    }

    // ── parse_response tests ──────────────────────────────────────────────────

    #[test]
    fn test_parse_response_normal() {
        let proto = OpenAiProtocol::new();
        let body = serde_json::json!({
            "choices": [{
                "message": { "content": "Hello, world!" },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        let resp = proto.parse_response(body).unwrap();
        assert_eq!(resp.content_blocks.len(), 1);
        assert!(
            matches!(resp.content_blocks[0], RawContentBlock::Text(ref s) if s == "Hello, world!")
        );
        assert_eq!(resp.usage.prompt_tokens, 10);
        assert_eq!(resp.usage.completion_tokens, 5);
        assert_eq!(resp.usage.total_tokens, Some(15));
        assert_eq!(resp.finish_reason, Some("stop".to_string()));
    }

    #[test]
    fn test_parse_response_empty_choices() {
        let proto = OpenAiProtocol::new();
        let body = serde_json::json!({ "choices": [] });
        let resp = proto.parse_response(body).unwrap();
        assert!(resp.content_blocks.is_empty());
        assert_eq!(resp.usage.prompt_tokens, 0);
        assert_eq!(resp.usage.completion_tokens, 0);
    }

    #[test]
    fn test_parse_response_missing_usage_defaults() {
        let proto = OpenAiProtocol::new();
        let body = serde_json::json!({ "choices": [{ "message": { "content": "Hi" } }] });
        let resp = proto.parse_response(body).unwrap();
        assert_eq!(resp.usage.prompt_tokens, 0);
        assert_eq!(resp.usage.completion_tokens, 0);
        assert!(resp.usage.total_tokens.is_none());
    }

    // ── decorate_headers tests ────────────────────────────────────────────────

    #[test]
    fn test_decorate_headers_bearer() {
        std::env::remove_var("OPENAI_API_KEY");
        let proto = OpenAiProtocol::new();
        let mut headers = HeaderMap::new();
        proto.decorate_headers(&mut headers).unwrap();

        let auth = headers.get(AUTHORIZATION).unwrap();
        assert!(auth.to_str().unwrap().starts_with("Bearer "));
    }

    #[test]
    fn test_decorate_headers_content_type() {
        let proto = OpenAiProtocol::new();
        let mut headers = HeaderMap::new();
        proto.decorate_headers(&mut headers).unwrap();

        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
            "application/json"
        );
    }
}
