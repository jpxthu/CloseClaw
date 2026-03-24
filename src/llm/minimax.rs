//! MiniMax LLM Provider

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, Usage};

/// MiniMax API endpoint
const MINIMAX_API_URL: &str = "https://api.minimax.chat/v1/chat/completions";

/// MiniMax API request body
#[derive(Debug, Serialize)]
struct MiniMaxRequest<'a> {
    model: &'a str,
    messages: &'a [crate::llm::Message],
    temperature: f32,
    max_tokens: Option<u32>,
}

/// MiniMax API response body
#[derive(Debug, Deserialize)]
struct MiniMaxResponse {
    choices: Vec<MiniMaxChoice>,
    usage: MiniMaxUsage,
    model: String,
}

#[derive(Debug, Deserialize)]
struct MiniMaxChoice {
    message: MiniMaxMessage,
}

#[derive(Debug, Deserialize)]
struct MiniMaxMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct MiniMaxUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

pub struct MiniMaxProvider {
    api_key: String,
    base_url: String,
    http_client: Client,
}

impl MiniMaxProvider {
    pub fn new(api_key: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("MiniMaxProvider: failed to build HTTP client");

        Self {
            api_key,
            base_url: MINIMAX_API_URL.to_string(),
            http_client,
        }
    }

    /// Create a new provider from environment variable MINIMAX_API_KEY.
    pub fn from_env() -> Option<Self> {
        std::env::var("MINIMAX_API_KEY").ok().map(Self::new)
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
        let req_body = MiniMaxRequest {
            model: &request.model,
            messages: &request.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let response = self
            .http_client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(if status.as_u16() == 401 {
                LLMError::AuthFailed(format!("MiniMax auth failed: {}", body))
            } else if status.as_u16() == 429 {
                LLMError::RateLimitExceeded
            } else if status.as_u16() == 400 {
                LLMError::InvalidRequest(format!("MiniMax invalid request: {}", body))
            } else {
                LLMError::ApiError(format!("MiniMax API error ({}): {}", status, body))
            });
        }

        let api_resp: MiniMaxResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(format!("failed to parse MiniMax response: {}", e)))?;

        let content = api_resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: api_resp.model,
            usage: Usage {
                prompt_tokens: api_resp.usage.prompt_tokens,
                completion_tokens: api_resp.usage.completion_tokens,
                total_tokens: api_resp.usage.total_tokens,
            },
        })
    }
}
