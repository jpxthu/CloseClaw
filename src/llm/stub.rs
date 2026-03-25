//! Stub LLM Provider - Returns fixed responses for testing

use async_trait::async_trait;
use std::sync::Arc;

use super::{ChatRequest, ChatResponse, LLMError, LLMProvider, Message, Usage};

/// A stub LLM provider that returns fixed responses.
/// Always returns `is_stub() == true` so callers can detect test configurations.
#[derive(Debug, Clone, Default)]
pub struct StubProvider {
    /// Fixed response content returned by `chat()`
    response: String,
    /// Fixed model name in response
    model: String,
}

impl StubProvider {
    /// Create a new StubProvider with default response ("stub response")
    pub fn new() -> Self {
        Self {
            response: "stub response".to_string(),
            model: "stub-model".to_string(),
        }
    }

    /// Create a new StubProvider with a custom response
    pub fn with_response(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            model: "stub-model".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for StubProvider {
    fn name(&self) -> &str {
        "stub"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        // Log the request for test inspection
        eprintln!("[StubProvider] chat called with model={}", request.model);
        eprintln!(
            "[StubProvider] messages count={}",
            request.messages.len()
        );

        let prompt_tokens = request
            .messages
            .iter()
            .map(|m| m.content.len() as u32 / 4)
            .sum();

        Ok(ChatResponse {
            content: self.response.clone(),
            model: self.model.clone(),
            usage: Usage {
                prompt_tokens,
                completion_tokens: self.response.len() as u32 / 4,
                total_tokens: prompt_tokens + self.response.len() as u32 / 4,
            },
        })
    }

    fn models(&self) -> Vec<&str> {
        vec!["stub-model"]
    }

    fn is_stub(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_provider_is_stub() {
        let provider = StubProvider::new();
        assert!(provider.is_stub());
    }

    #[tokio::test]
    async fn test_stub_provider_name() {
        let provider = StubProvider::new();
        assert_eq!(provider.name(), "stub");
    }

    #[tokio::test]
    async fn test_stub_provider_models() {
        let provider = StubProvider::new();
        assert_eq!(provider.models(), vec!["stub-modem"]);
    }

    #[tokio::test]
    async fn test_stub_provider_chat_returns_fixed_response() {
        let provider = StubProvider::new();
        let request = ChatRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            temperature: 0.7,
            max_tokens: None,
        };

        let response = provider.chat(request).await.unwrap();
        assert_eq!(response.content, "stub response");
        assert_eq!(response.model, "stub-model");
        assert!(response.usage.total_tokens > 0);
    }

    #[tokio::test]
    async fn test_stub_provider_custom_response() {
        let provider = StubProvider::with_response("custom test response");
        let request = ChatRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "test".to_string(),
            }],
            temperature: 0.0,
            max_tokens: Some(100),
        };

        let response = provider.chat(request).await.unwrap();
        assert_eq!(response.content, "custom test response");
    }
}
