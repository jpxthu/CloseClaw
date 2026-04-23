//! Anthropic LLM Provider

use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider};
use async_trait::async_trait;

#[allow(dead_code)]
pub struct AnthropicProvider {
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<&str> {
        vec!["claude-3-opus", "claude-3-sonnet", "claude-3-haiku"]
    }

    fn is_stub(&self) -> bool {
        true
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LLMError> {
        Err(LLMError::ApiError(
            "Anthropic provider is a stub — implement real API to enable LLM calls".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_provider_new() {
        let provider = AnthropicProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_anthropic_provider_name() {
        let provider = AnthropicProvider::new("key".to_string());
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_anthropic_provider_models() {
        let provider = AnthropicProvider::new("key".to_string());
        let models = provider.models();
        assert_eq!(
            models,
            vec!["claude-3-opus", "claude-3-sonnet", "claude-3-haiku"]
        );
    }

    #[test]
    fn test_anthropic_is_stub() {
        let provider = AnthropicProvider::new("key".to_string());
        assert!(provider.is_stub());
    }

    #[tokio::test]
    async fn test_anthropic_chat_returns_error() {
        let provider = AnthropicProvider::new("key".to_string());
        let request = ChatRequest {
            model: "claude-3-opus".to_string(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
        };
        let result = provider.chat(request).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            LLMError::ApiError(msg) => assert!(msg.contains("stub")),
            _ => panic!("Expected ApiError"),
        }
    }
}
