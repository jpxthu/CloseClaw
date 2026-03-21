//! Anthropic LLM Provider

use async_trait::async_trait;
use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, Usage};

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

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        // Stub - would call Anthropic API
        Ok(ChatResponse {
            content: format!("[Anthropic stub] Response to: {}", request.messages.last().map(|m| m.content.as_str()).unwrap_or("")),
            model: request.model,
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
        })
    }
}
