//! Stub LLM Provider - Returns fixed responses for testing

use reqwest::header::HeaderMap;
use reqwest::Client;
use std::sync::OnceLock;
use tokio::sync::mpsc;

use async_trait::async_trait;

use super::provider::{Provider, Result, SseStream};
use super::types::{InternalRequest, ProtocolId, RawSseChunk};

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
    ) -> Result<serde_json::Value> {
        // Log the request for test inspection
        eprintln!("[StubProvider] send called with model={}", request.model);
        eprintln!("[StubProvider] messages count={}", request.messages.len());

        let prompt_tokens: u32 = request
            .messages
            .iter()
            .map(|m| m.content.len() as u32 / 4)
            .sum();
        let completion_tokens = self.response.len() as u32 / 4;

        Ok(serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": self.response
                },
                "finish_reason": null
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens
            }
        }))
    }

    async fn send_streaming(
        &self,
        request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream> {
        let response = self.send(request, body).await?;
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            // Extract content from the JSON response
            if let Some(choices) = response.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    if let Some(message) = choice.get("message") {
                        if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                            let _ = tx
                                .send(RawSseChunk {
                                    event_type: "message".into(),
                                    data: content.to_string(),
                                })
                                .await;
                        }
                    }
                }
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
        // Verify raw JSON structure
        let choices = response["choices"].as_array().unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(
            choices[0]["message"]["content"].as_str().unwrap(),
            "stub response"
        );
        assert!(response["usage"]["total_tokens"].as_u64().unwrap() > 0);
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
        assert_eq!(
            response["choices"][0]["message"]["content"]
                .as_str()
                .unwrap(),
            "custom test response"
        );
    }
}
