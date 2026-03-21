//! MiniMax LLM Provider

use async_trait::async_trait;
use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, Usage};

#[allow(dead_code)]
pub struct MiniMaxProvider {
    api_key: String,
    base_url: String,
}

impl MiniMaxProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.minimax.chat/v1".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for MiniMaxProvider {
    fn name(&self) -> &str {
        "minimax"
    }

    fn models(&self) -> Vec<&str> {
        vec!["MiniMax-M2", "MiniMax-M2.1", "MiniMax-M2.5", "MiniMax-M2.7"]
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        // Stub - would call MiniMax API
        Ok(ChatResponse {
            content: format!("[MiniMax stub] Response to: {}", request.messages.last().map(|m| m.content.as_str()).unwrap_or("")),
            model: request.model,
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
        })
    }
}
