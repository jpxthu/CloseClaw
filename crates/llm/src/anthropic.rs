//! Anthropic LLM Provider — pure HTTP transport for the Anthropic Messages API.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use serde::Deserialize;
use std::sync::OnceLock;

use crate::provider::{Provider, ProviderError, Result, SseStream};
use crate::types::{InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawUsage};

pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    client: Client,
    supported_protocols: Vec<ProtocolId>,
}

// ── Raw Anthropic API response types ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    usage: AnthropicUsage,
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.anthropic.com".to_string(),
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("anthropic")],
        }
    }

    pub fn new_with_base_url(api_key: String, base_url: &str) -> Self {
        Self {
            api_key,
            base_url: base_url.to_string(),
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("anthropic")],
        }
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }

    fn map_status_error(status: reqwest::StatusCode, body: String) -> ProviderError {
        ProviderError::Legacy(format!("Anthropic API error {}: {}", status, body))
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn supported_protocols(&self) -> &[ProtocolId] {
        &self.supported_protocols
    }

    fn http_client(&self) -> &Client {
        &self.client
    }

    fn default_headers(&self) -> &HeaderMap {
        static EMPTY: OnceLock<HeaderMap> = OnceLock::new();
        EMPTY.get_or_init(HeaderMap::new)
    }

    async fn send(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<InternalResponse> {
        let url = self.messages_url();

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key)
                .unwrap_or_else(|_| HeaderValue::from_static("invalid")),
        );
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let anthropic_resp: AnthropicResponse =
            response.json().await.map_err(ProviderError::Reqwest)?;

        let content_blocks: Vec<RawContentBlock> = anthropic_resp
            .content
            .into_iter()
            .map(|block| match block {
                AnthropicContentBlock::Text { text } => RawContentBlock::Text(text),
                AnthropicContentBlock::Thinking {
                    thinking,
                    signature,
                } => RawContentBlock::Thinking {
                    thinking,
                    signature,
                },
                AnthropicContentBlock::ToolUse { id, name, input } => RawContentBlock::ToolUse {
                    id,
                    name,
                    input: input.to_string(),
                },
            })
            .collect();

        Ok(InternalResponse {
            content_blocks,
            usage: RawUsage {
                prompt_tokens: anthropic_resp.usage.input_tokens,
                completion_tokens: anthropic_resp.usage.output_tokens,
                total_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: anthropic_resp.stop_reason,
        })
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> Result<SseStream> {
        Err(ProviderError::Legacy(
            "Anthropic streaming not yet implemented".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InternalMessage;
    use closeclaw_session::persistence::ReasoningLevel;
    use mockito::Server;

    #[test]
    fn test_anthropic_provider_new() {
        let provider = AnthropicProvider::new("test-key".to_string());
        assert_eq!(provider.id(), "anthropic");
        assert_eq!(provider.api_key(), "test-key");
    }

    #[test]
    fn test_anthropic_provider_id() {
        let provider = AnthropicProvider::new("key".to_string());
        assert_eq!(provider.id(), "anthropic");
    }

    #[test]
    fn test_anthropic_provider_base_url() {
        let provider = AnthropicProvider::new("key".to_string());
        assert_eq!(provider.base_url(), "https://api.anthropic.com");
    }

    #[test]
    fn test_anthropic_provider_base_url_custom() {
        let provider =
            AnthropicProvider::new_with_base_url("key".to_string(), "https://custom.api.com");
        assert_eq!(provider.base_url(), "https://custom.api.com");
    }

    #[test]
    fn test_anthropic_provider_supported_protocols() {
        let provider = AnthropicProvider::new("key".to_string());
        let protocols = provider.supported_protocols();
        assert_eq!(protocols.len(), 1);
        assert_eq!(protocols[0].as_str(), "anthropic");
    }

    fn make_request() -> InternalRequest {
        InternalRequest {
            model: "claude-3-opus".into(),
            messages: vec![InternalMessage {
                role: "user".into(),
                content: "hello".into(),
                ..Default::default()
            }],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        }
    }

    // ── send() success tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_success() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_body(
                r#"{
                    "content": [{"type": "text", "text": "Hello there!"}],
                    "usage": {"input_tokens": 10, "output_tokens": 5},
                    "stop_reason": "end_turn"
                }"#,
            )
            .create_async()
            .await;

        let provider = AnthropicProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = make_request();
        let response = provider
            .send(
                request,
                serde_json::json!({
                    "model": "claude-3-opus",
                    "messages": [{"role": "user", "content": "hello"}]
                }),
            )
            .await
            .unwrap();
        mock.assert_async().await;

        assert_eq!(response.content_blocks.len(), 1);
        match &response.content_blocks[0] {
            RawContentBlock::Text(s) => assert_eq!(s, "Hello there!"),
            other => panic!("Expected Text block, got: {:?}", other),
        }
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.completion_tokens, 5);
        assert_eq!(response.usage.total_tokens, None);
        assert_eq!(response.finish_reason.as_deref(), Some("end_turn"));
    }

    // ── send() with thinking block ───────────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_with_thinking() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_body(
                r#"{
                    "content": [
                        {"type": "thinking", "thinking": "Let me think..."},
                        {"type": "text", "text": "The answer is 42."}
                    ],
                    "usage": {"input_tokens": 20, "output_tokens": 15},
                    "stop_reason": "end_turn"
                }"#,
            )
            .create_async()
            .await;

        let provider = AnthropicProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = make_request();
        let response = provider
            .send(
                request,
                serde_json::json!({
                    "model": "claude-3-opus",
                    "messages": []
                }),
            )
            .await
            .unwrap();
        mock.assert_async().await;

        assert_eq!(response.content_blocks.len(), 2);
        match &response.content_blocks[0] {
            RawContentBlock::Thinking {
                thinking,
                signature,
            } => {
                assert_eq!(thinking, "Let me think...");
                assert!(signature.is_none());
            }
            other => {
                panic!("Expected Thinking block, got: {:?}", other)
            }
        }
        match &response.content_blocks[1] {
            RawContentBlock::Text(s) => {
                assert_eq!(s, "The answer is 42.")
            }
            other => panic!("Expected Text block, got: {:?}", other),
        }
        assert_eq!(response.usage.prompt_tokens, 20);
        assert_eq!(response.usage.completion_tokens, 15);
    }

    // ── send() empty content ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_empty_content() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_body(
                r#"{
                    "content": [],
                    "usage": {"input_tokens": 5, "output_tokens": 0},
                    "stop_reason": "end_turn"
                }"#,
            )
            .create_async()
            .await;

        let provider = AnthropicProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = make_request();
        let response = provider.send(request, serde_json::json!({})).await.unwrap();
        mock.assert_async().await;

        assert!(response.content_blocks.is_empty());
        assert_eq!(response.usage.prompt_tokens, 5);
        assert_eq!(response.usage.completion_tokens, 0);
        assert_eq!(response.finish_reason.as_deref(), Some("end_turn"));
    }

    // ── send() auth error ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_auth_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(401)
            .with_body(r#"{"error": {"message": "invalid API key"}}"#)
            .create_async()
            .await;

        let provider = AnthropicProvider::new_with_base_url("bad-key".to_string(), &server.url());
        let request = make_request();
        let result = provider.send(request, serde_json::json!({})).await;
        mock.assert_async().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Legacy(msg) => assert!(msg.contains("401")),
            other => {
                panic!("Expected Legacy error for 401, got: {:?}", other)
            }
        }
    }

    // ── send() rate limit ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_rate_limit() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(429)
            .with_body(r#"{"error": {"message": "rate limited"}}"#)
            .create_async()
            .await;

        let provider = AnthropicProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = make_request();
        let result = provider.send(request, serde_json::json!({})).await;
        mock.assert_async().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Legacy(msg) => assert!(msg.contains("429")),
            other => {
                panic!("Expected Legacy error for 429, got: {:?}", other)
            }
        }
    }

    // ── send() server error ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_server_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(500)
            .with_body(r#"{"error": {"message": "internal error"}}"#)
            .create_async()
            .await;

        let provider = AnthropicProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = make_request();
        let result = provider.send(request, serde_json::json!({})).await;
        mock.assert_async().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Legacy(msg) => assert!(msg.contains("500")),
            other => {
                panic!("Expected Legacy error for 500, got: {:?}", other)
            }
        }
    }

    // ── send() thinking with signature ───────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_thinking_with_signature() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_body(
                r#"{
                    "content": [
                        {"type": "thinking", "thinking": "reasoning...", "signature": "sig123"}
                    ],
                    "usage": {"input_tokens": 10, "output_tokens": 20},
                    "stop_reason": "end_turn"
                }"#,
            )
            .create_async()
            .await;

        let provider = AnthropicProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = make_request();
        let response = provider.send(request, serde_json::json!({})).await.unwrap();
        mock.assert_async().await;

        assert_eq!(response.content_blocks.len(), 1);
        match &response.content_blocks[0] {
            RawContentBlock::Thinking {
                thinking,
                signature,
            } => {
                assert_eq!(thinking, "reasoning...");
                assert_eq!(signature.as_deref(), Some("sig123"));
            }
            other => {
                panic!("Expected Thinking block, got: {:?}", other)
            }
        }
    }

    // ── send_streaming() stub ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_anthropic_send_streaming_not_implemented() {
        let provider = AnthropicProvider::new("key".to_string());
        let request = make_request();
        let result = provider
            .send_streaming(request, serde_json::json!({}))
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Legacy(msg) => {
                assert!(msg.contains("not yet implemented"))
            }
            other => {
                panic!("Expected Legacy error, got: {:?}", other)
            }
        }
    }
}
