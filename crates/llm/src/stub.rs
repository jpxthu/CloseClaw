//! Stub LLM Provider - Returns fixed responses for testing

use reqwest::header::HeaderMap;
use reqwest::Client;
use std::sync::OnceLock;
use tokio::sync::mpsc;

use async_trait::async_trait;

use super::provider::{Provider, Result, SseStream};
use super::types::{
    InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage,
};

/// A stub LLM provider that returns fixed responses.
/// Always returns `id() == "stub"` so callers can detect test configurations.
#[derive(Debug, Clone, Default)]
pub struct StubProvider {
    /// Fixed response content returned by `send()`
    response: String,
    /// HTTP client (satisfies the `http_client()` contract; unused by stub)
    client: Client,
}

impl StubProvider {
    /// Create a new StubProvider with default response ("stub response")
    pub fn new() -> Self {
        Self {
            response: "stub response".to_string(),
            client: Client::new(),
        }
    }

    /// Create a new StubProvider with a custom response
    pub fn with_response(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            client: Client::new(),
        }
    }
}

#[async_trait]
impl Provider for StubProvider {
    fn id(&self) -> &str {
        "stub"
    }

    fn base_url(&self) -> &str {
        ""
    }

    fn api_key(&self) -> &str {
        ""
    }

    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
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
        request: InternalRequest,
        _body: serde_json::Value,
    ) -> Result<InternalResponse> {
        // Log the request for test inspection
        eprintln!("[StubProvider] send called with model={}", request.model);
        eprintln!("[StubProvider] messages count={}", request.messages.len());

        let prompt_tokens = request
            .messages
            .iter()
            .map(|m| m.content.len() as u32 / 4)
            .sum();

        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text(self.response.clone())],
            usage: RawUsage {
                prompt_tokens,
                completion_tokens: self.response.len() as u32 / 4,
                total_tokens: Some(prompt_tokens + self.response.len() as u32 / 4),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }

    async fn send_streaming(
        &self,
        request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream> {
        let response = self.send(request, body).await?;
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            for block in &response.content_blocks {
                let chunk = match block {
                    RawContentBlock::Text(s) => RawSseChunk {
                        event_type: "message".into(),
                        data: s.clone(),
                    },
                    _ => continue,
                };
                let _ = tx.send(chunk).await;
            }
            // Send done event
            let done = serde_json::json!({"type": "message_end"});
            let _ = tx
                .send(RawSseChunk {
                    event_type: "message".into(),
                    data: done.to_string(),
                })
                .await;
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::InternalMessage;
    use super::*;

    #[test]
    fn test_stub_provider_is_stub() {
        let provider = StubProvider::new();
        assert_eq!(provider.id(), "stub");
    }

    #[test]
    fn test_stub_provider_name() {
        let provider = StubProvider::new();
        assert_eq!(provider.id(), "stub");
    }

    #[tokio::test]
    async fn test_stub_provider_chat_returns_fixed_response() {
        let provider = StubProvider::new();
        let request = InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                ..Default::default()
            }],
            temperature: 0.7,
            max_tokens: None,
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: closeclaw_session::persistence::ReasoningLevel::default(),
            turn_count: None,
        };

        let response = provider
            .send(request, serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(response.content_blocks.len(), 1);
        match &response.content_blocks[0] {
            RawContentBlock::Text(s) => assert_eq!(s, "stub response"),
            other => panic!("Expected Text block, got: {:?}", other),
        }
        assert!(response.usage.total_tokens.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_stub_provider_custom_response() {
        let provider = StubProvider::with_response("custom test response");
        let request = InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: "test".to_string(),
                ..Default::default()
            }],
            temperature: 0.0,
            max_tokens: Some(100),
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            tools: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: closeclaw_session::persistence::ReasoningLevel::default(),
            turn_count: None,
        };

        let response = provider
            .send(request, serde_json::Value::Null)
            .await
            .unwrap();
        match &response.content_blocks[0] {
            RawContentBlock::Text(s) => assert_eq!(s, "custom test response"),
            other => panic!("Expected Text block, got: {:?}", other),
        }
    }
}
