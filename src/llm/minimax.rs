//! MiniMax LLM Provider

use async_trait::async_trait;
use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, Message, Usage};
use serde::{Deserialize, Serialize};

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

    fn chat_url(&self) -> String {
        format!("{}/text/chatcompletion_v2", self.base_url)
    }
}

#[derive(Debug, Serialize)]
struct MiniMaxRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct MiniMaxResponse {
    id: String,
    model: String,
    choices: Vec<MiniMaxChoice>,
    usage: MiniMaxUsage,
}

#[derive(Debug, Deserialize)]
struct MiniMaxChoice {
    finish_reason: String,
    message: MiniMaxMessage,
}

#[derive(Debug, Deserialize)]
struct MiniMaxMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct MiniMaxUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
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
        let url = self.chat_url();

        let body = MiniMaxRequest {
            model: &request.model,
            messages: &request.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(LLMError::AuthFailed(format!("MiniMax API auth failed: {}", status)));
            }
            if status.as_u16() == 429 {
                return Err(LLMError::RateLimitExceeded);
            }
            let body = response.text().await.unwrap_or_default();
            return Err(LLMError::ApiError(format!("MiniMax API error {}: {}", status, body)));
        }

        let mm_resp: MiniMaxResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(format!("failed to parse MiniMax response: {}", e)))?;

        let choice = mm_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LLMError::ApiError("no choices in MiniMax response".to_string()))?;

        Ok(ChatResponse {
            content: choice.message.content,
            model: mm_resp.model,
            usage: Usage {
                prompt_tokens: mm_resp.usage.prompt_tokens,
                completion_tokens: mm_resp.usage.completion_tokens,
                total_tokens: mm_resp.usage.total_tokens,
            },
        })
    }
}
