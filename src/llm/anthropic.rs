//! Anthropic LLM Provider

use async_trait::async_trait;
use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider};

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
