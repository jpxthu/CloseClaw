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

#[async_trait]
impl ChatProtocol for AnthropicProtocol {
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
                            Some("thinking") => item
                                .get("thinking")
                                .and_then(|v| v.as_str())
                                .map(|s| RawContentBlock::Thinking(s.to_string())),
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

    RawUsage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::InternalMessage;

    fn make_request() -> InternalRequest {
        InternalRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            temperature: 0.7,
            max_tokens: Some(1024),
            stream: false,
            extra_body: Default::default(),
        }
    }

    // ── build_request tests ───────────────────────────────────────────────────

    #[test]
    fn test_build_request_basic() {
        let proto = AnthropicProtocol::new();
        let request = make_request();
        let body = proto.build_request(&request).unwrap();

        assert_eq!(body.get("model").unwrap(), "claude-3-5-sonnet-20241022");
        assert!(body.get("messages").unwrap().is_array());
        assert_eq!(body.get("max_tokens").unwrap(), &serde_json::json!(1024));
    }

    #[test]
    fn test_build_request_no_max_tokens() {
        let proto = AnthropicProtocol::new();
        let mut request = make_request();
        request.max_tokens = None;
        let body = proto.build_request(&request).unwrap();
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn test_build_request_stream_flag() {
        let proto = AnthropicProtocol::new();
        let mut request = make_request();
        request.stream = true;
        let body = proto.build_request(&request).unwrap();
        // Anthropic /v1/messages does not have a `stream` bool in the body
        // for the HTTP POST; streaming uses a different endpoint (SSE).
        assert!(body.get("stream").is_none());
    }

    // ── parse_response tests ──────────────────────────────────────────────────

    #[test]
    fn test_parse_response_normal() {
        let proto = AnthropicProtocol::new();
        let body = serde_json::json!({
            "content": [
                {"type": "text", "text": "Hello, world!"}
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
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
        assert_eq!(resp.finish_reason, Some("end_turn".to_string()));
    }

    #[test]
    fn test_parse_response_thinking_block() {
        let proto = AnthropicProtocol::new();
        let body = serde_json::json!({
            "content": [
                {"type": "thinking", "thinking": "Let me think..."},
                {"type": "text", "text": "Final answer."}
            ],
            "stop_reason": "end_turn"
        });

        let resp = proto.parse_response(body).unwrap();
        assert_eq!(resp.content_blocks.len(), 2);
        assert!(
            matches!(resp.content_blocks[0], RawContentBlock::Thinking(ref s) if s == "Let me think...")
        );
        assert!(
            matches!(resp.content_blocks[1], RawContentBlock::Text(ref s) if s == "Final answer.")
        );
    }

    #[test]
    fn test_parse_response_empty_content() {
        let proto = AnthropicProtocol::new();
        let body = serde_json::json!({ "content": [], "stop_reason": "end_turn" });
        let resp = proto.parse_response(body).unwrap();
        assert!(resp.content_blocks.is_empty());
        assert_eq!(resp.usage.prompt_tokens, 0);
        assert_eq!(resp.usage.completion_tokens, 0);
    }

    #[test]
    fn test_parse_response_missing_usage_defaults() {
        let proto = AnthropicProtocol::new();
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "Hi"}],
            "stop_reason": "end_turn"
        });
        let resp = proto.parse_response(body).unwrap();
        assert_eq!(resp.usage.prompt_tokens, 0);
        assert_eq!(resp.usage.completion_tokens, 0);
        assert!(resp.usage.total_tokens.is_none());
    }

    // ── decorate_headers tests ────────────────────────────────────────────────

    #[test]
    fn test_decorate_headers_api_key() {
        std::env::remove_var("ANTHROPIC_API_KEY");
        let proto = AnthropicProtocol::new();
        let mut headers = HeaderMap::new();
        proto.decorate_headers(&mut headers).unwrap();

        let key_header = headers.get("x-api-key").unwrap();
        assert_eq!(key_header.to_str().unwrap(), "");
    }

    #[test]
    fn test_decorate_headers_anthropic_version() {
        let proto = AnthropicProtocol::new();
        let mut headers = HeaderMap::new();
        proto.decorate_headers(&mut headers).unwrap();

        assert_eq!(
            headers.get("anthropic-version").unwrap().to_str().unwrap(),
            "2023-06-01"
        );
    }

    #[test]
    fn test_decorate_headers_content_type() {
        let proto = AnthropicProtocol::new();
        let mut headers = HeaderMap::new();
        proto.decorate_headers(&mut headers).unwrap();

        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
            "application/json"
        );
    }
}
