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
    /// MiniMax thinking content (present for -think models).
    /// Contains raw thinking output which should NOT be shown to the user.
    #[serde(default)]
    thinking: Option<String>,
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

    /// Map HTTP status code to the appropriate LLM error.
    fn map_status_error(status: reqwest::StatusCode, body: String) -> LLMError {
        match status.as_u16() {
            401 => LLMError::AuthFailed(format!("MiniMax auth failed: {}", body)),
            429 => LLMError::RateLimitExceeded,
            400 => LLMError::InvalidRequest(format!("MiniMax invalid request: {}", body)),
            _ => LLMError::ApiError(format!("MiniMax API error ({}): {}", status, body)),
        }
    }

    /// Strip MiniMax thinking tags from content.
    ///
    /// MiniMax thinking models return reasoning wrapped in XML tags:
    /// - `<think>...</think>` in the content field (visible thinking text)
    ///
    /// These tags and their content are NOT meant for end users and must be
    /// stripped before returning the response.
    fn strip_thinking_tags(content: &str) -> String {
        // Remove thinking tags and their content, skipping orphaned closing tags
        let bytes = content.as_bytes();
        let mut result = Vec::new();
        let mut i = 0;

        while i < bytes.len() {
            // If we see a closing tag with no opening tag before it, skip past it
            if bytes[i..].starts_with(b"</think>") {
                i += 8; // skip the closing tag
                continue;
            }
            // If we see an opening tag, find its matching closing tag and skip both
            if bytes[i..].starts_with("<think>".as_bytes()) {
                if let Some(end) = content[i..].find("</think>") {
                    i = end + 8; // skip past the closing tag
                    continue;
                }
            }
            // Regular character, keep it
            result.push(bytes[i]);
            i += 1;
        }

        String::from_utf8(result)
            .unwrap_or_default()
            .trim()
            .to_string()
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
            return Err(Self::map_status_error(status, body));
        }

        let api_resp: MiniMaxResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(format!("failed to parse MiniMax response: {}", e)))?;

        let msg = api_resp
            .choices
            .first()
            .map(|c| &c.message)
            .ok_or_else(|| LLMError::ApiError("no choices in MiniMax response".to_string()))?;

        // Strip thinking tags from content before returning to user
        let content = Self::strip_thinking_tags(&msg.content);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_thinking_tags_simple() {
        let input = "<think>Let me think about this...</think>Hello, world!";
        let output = MiniMaxProvider::strip_thinking_tags(input);
        assert_eq!(output, "Hello, world!");
    }

    #[test]
    fn test_strip_thinking_tags_no_tags() {
        let input = "Hello, world!";
        let output = MiniMaxProvider::strip_thinking_tags(input);
        assert_eq!(output, "Hello, world!");
    }

    #[test]
    fn test_strip_thinking_tags_only_tags() {
        let input = "<think>thinking...</think>";
        let output = MiniMaxProvider::strip_thinking_tags(input);
        assert_eq!(output, "");
    }

    #[test]
    fn test_strip_thinking_tags_multiple() {
        let input = "<think>first...</think>Hello</think>World";
        let output = MiniMaxProvider::strip_thinking_tags(input);
        assert_eq!(output, "HelloWorld");
    }
}
