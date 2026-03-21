//! OpenAI LLM Provider

use async_trait::async_trait;
use std::sync::Arc;
use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, Usage};

pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn models(&self) -> Vec<&str> {
        vec!["gpt-4", "gpt-4-turbo", "gpt-3.5-turbo"]
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        // Stub - would call OpenAI API
        Ok(ChatResponse {
            content: format!("[OpenAI stub] Response to: {}", request.messages.last().map(|m| m.content.as_str()).unwrap_or("")),
            model: request.model,
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
        })
    }
}
