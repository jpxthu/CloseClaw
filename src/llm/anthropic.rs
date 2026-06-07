//! Anthropic LLM Provider

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use std::sync::OnceLock;

use crate::llm::provider::{Provider, ProviderError, Result, SseStream};
use crate::llm::types::{InternalRequest, InternalResponse, ProtocolId};

pub struct AnthropicProvider {
    api_key: String,
    client: Client,
    supported_protocols: Vec<ProtocolId>,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("anthropic")],
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn base_url(&self) -> &str {
        ""
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
        _body: serde_json::Value,
    ) -> Result<InternalResponse> {
        Err(ProviderError::Legacy(
            "Anthropic provider is a stub — implement real API to enable LLM calls".into(),
        ))
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> Result<SseStream> {
        Err(ProviderError::Legacy(
            "Anthropic provider is a stub — implement real API to enable LLM calls".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_anthropic_provider_supported_protocols() {
        let provider = AnthropicProvider::new("key".to_string());
        let protocols = provider.supported_protocols();
        assert_eq!(protocols.len(), 1);
        assert_eq!(protocols[0].as_str(), "anthropic");
    }

    #[tokio::test]
    async fn test_anthropic_send_returns_error() {
        let provider = AnthropicProvider::new("key".to_string());
        let request = InternalRequest {
            model: "claude-3-opus".into(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: crate::session::persistence::ReasoningLevel::default(),
            turn_count: None,
        };
        let result = provider.send(request, serde_json::Value::Null).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProviderError::Legacy(msg) => assert!(msg.contains("stub")),
            other => panic!("Expected Legacy error, got: {:?}", other),
        }
    }
}
