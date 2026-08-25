//! OpenAI LLM Provider — pure HTTP transport for the OpenAI Chat Completions API.
//!
//! This provider carries configuration (URL, credentials, HTTP client)
//! and performs the HTTP request/response cycle.
//! Serialization/deserialization is handled by `OpenAiProtocol`.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::HeaderMap;
use reqwest::Client;
use std::sync::OnceLock;
use tokio::sync::mpsc;

use crate::provider::{Provider, ProviderError, Result, SseStream};
use crate::types::{InternalRequest, ProtocolId, RawSseChunk};

pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
    client: Client,
    supported_protocols: Vec<ProtocolId>,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("openai")],
        }
    }

    pub fn new_with_base_url(api_key: String, base_url: &str) -> Self {
        Self {
            api_key,
            base_url: base_url.to_string(),
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("openai")],
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

// ── Provider trait implementation ─────────────────────────────────────────────

#[async_trait]
impl Provider for OpenAIProvider {
    fn id(&self) -> &str {
        "openai"
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
    ) -> Result<serde_json::Value> {
        let url = self.chat_url();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::provider::map_http_error(status, body, None));
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(ProviderError::Reqwest)
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream> {
        let url = self.chat_url();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_after = crate::provider::parse_retry_after(response.headers());
            let body = response.text().await.unwrap_or_default();
            return Err(crate::provider::map_http_error(status, body, retry_after));
        }

        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(_) => break,
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE events (separated by \n\n)
                while let Some(pos) = buffer.find("\n\n") {
                    let event_block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    for line in event_block.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim().to_string();
                            if data == "[DONE]" {
                                return;
                            }
                            let _ = tx
                                .send(RawSseChunk {
                                    event_type: "message".into(),
                                    data,
                                })
                                .await;
                        }
                    }
                }
            }

            // Process any remaining data in buffer
            if !buffer.is_empty() {
                for line in buffer.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        let data = data.trim().to_string();
                        if data == "[DONE]" {
                            return;
                        }
                        let _ = tx
                            .send(RawSseChunk {
                                event_type: "message".into(),
                                data,
                            })
                            .await;
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InternalMessage;
    use closeclaw_session::persistence::ReasoningLevel;
    use mockito::Server;

    // Re-export for test use
    use crate::provider::Provider;

    // ── Provider accessor tests ───────────────────────────────────────────────

    #[test]
    fn test_openai_provider_id() {
        let provider = OpenAIProvider::new("test-key".to_string());
        assert_eq!(provider.id(), "openai");
    }

    #[test]
    fn test_openai_provider_base_url() {
        let provider = OpenAIProvider::new("key".to_string());
        assert_eq!(provider.base_url(), "https://api.openai.com/v1");
    }

    #[test]
    fn test_openai_provider_base_url_custom() {
        let provider =
            OpenAIProvider::new_with_base_url("key".to_string(), "https://custom.api.com");
        assert_eq!(provider.base_url(), "https://custom.api.com");
    }

    #[test]
    fn test_openai_provider_api_key() {
        let provider = OpenAIProvider::new("sk-test".to_string());
        assert_eq!(provider.api_key(), "sk-test");
    }

    #[test]
    fn test_openai_provider_supported_protocols() {
        let provider = OpenAIProvider::new("key".to_string());
        let protocols = provider.supported_protocols();
        assert_eq!(protocols.len(), 1);
        assert_eq!(protocols[0].as_str(), "openai");
    }

    // ── send() success test ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_openai_send_success() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_body(
                r#"{
                    "id": "chatcmpl-test",
                    "model": "gpt-4",
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "role": "assistant",
                            "content": "Hello there!"
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 5,
                        "total_tokens": 15
                    }
                }"#,
            )
            .create_async()
            .await;

        let provider = OpenAIProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
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
        };

        let response = provider
            .send(request, serde_json::json!({"model": "gpt-4", "messages": [{"role": "user", "content": "hi"}]}))
            .await
            .unwrap();
        mock.assert_async().await;

        // Verify raw JSON response structure
        assert_eq!(response["id"].as_str().unwrap(), "chatcmpl-test");
        assert_eq!(response["model"].as_str().unwrap(), "gpt-4");
        let choices = response["choices"].as_array().unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0]["finish_reason"].as_str().unwrap(), "stop");
        assert_eq!(
            choices[0]["message"]["content"].as_str().unwrap(),
            "Hello there!"
        );
        assert_eq!(response["usage"]["prompt_tokens"].as_u64().unwrap(), 10);
        assert_eq!(response["usage"]["completion_tokens"].as_u64().unwrap(), 5);
        assert_eq!(response["usage"]["total_tokens"].as_u64().unwrap(), 15);
    }

    // ── send() auth error test ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_openai_send_auth_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(401)
            .with_body(r#"{"error": {"message": "bad API key"}}"#)
            .create_async()
            .await;

        let provider = OpenAIProvider::new_with_base_url("bad-key".to_string(), &server.url());
        let request = InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            tools: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        };

        let result = provider.send(request, serde_json::json!({})).await;
        mock.assert_async().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Http { status_code, .. } => assert_eq!(status_code, 401),
            other => panic!("Expected Http error for 401, got: {:?}", other),
        }
    }

    // ── send() rate limit test ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_openai_send_rate_limit() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(429)
            .with_body(r#"{"error": {"message": "rate limited"}}"#)
            .create_async()
            .await;

        let provider = OpenAIProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            tools: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        };

        let result = provider.send(request, serde_json::json!({})).await;
        mock.assert_async().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Http { status_code, .. } => assert_eq!(status_code, 429),
            other => panic!("Expected Http error for 429, got: {:?}", other),
        }
    }

    // ── send_streaming() test ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_openai_send_streaming() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_body(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
                 data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n\
                 data: [DONE]\n\n",
            )
            .create_async()
            .await;

        let provider = OpenAIProvider::new_with_base_url("test-key".to_string(), &server.url());
        let request = InternalRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
            stream: true,
            extra_body: serde_json::Map::new(),
            tools: None,
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        };

        let mut stream = provider
            .send_streaming(
                request,
                serde_json::json!({"model": "gpt-4", "stream": true}),
            )
            .await
            .unwrap();
        mock.assert_async().await;

        let chunk1 = stream.recv().await.expect("should receive chunk 1");
        assert_eq!(chunk1.event_type, "message");
        assert!(chunk1.data.contains("Hello"));

        let chunk2 = stream.recv().await.expect("should receive chunk 2");
        assert_eq!(chunk2.event_type, "message");
        assert!(chunk2.data.contains("world"));

        let done = stream.recv().await;
        assert!(done.is_none(), "channel should be closed after [DONE]");
    }
}
