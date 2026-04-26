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
    #[serde(default)]
    choices: Option<Vec<MiniMaxChoice>>,
    #[serde(default)]
    usage: Option<MiniMaxUsage>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    base_resp: Option<MiniMaxBaseResp>,
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
    /// MiniMax reasoning content for M2.5/M2.7 models.
    /// When content is empty, the visible reply is in this field.
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct MiniMaxUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
    #[serde(default)]
    completion_tokens_details: Option<MiniMaxCompletionTokensDetails>,
}

/// MiniMax completion tokens details
#[derive(Debug, Deserialize)]
struct MiniMaxCompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u32>,
}

/// MiniMax base response (business error status)
#[derive(Debug, Deserialize)]
struct MiniMaxBaseResp {
    status_code: i32,
    status_msg: String,
}

pub struct MiniMaxProvider {
    api_key: String,
    base_url: String,
    http_client: Client,
}

impl MiniMaxProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, MINIMAX_API_URL.to_string())
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(std::env::var("MINIMAX_API_KEY").ok()?))
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            api_key,
            base_url,
            http_client,
        }
    }

    fn map_status_error(status: reqwest::StatusCode, body: String) -> LLMError {
        match status.as_u16() {
            401 | 403 => LLMError::AuthFailed(body),
            404 => LLMError::ModelNotFound(body),
            422 => LLMError::InvalidRequest(body),
            429 => LLMError::RateLimitExceeded,
            _ => LLMError::ApiError(format!("unexpected status {}: {}", status, body)),
        }
    }

    /// Map MiniMax internal status_code to LLMError
    fn map_base_resp_error(status_code: i32, status_msg: &str) -> LLMError {
        match status_code {
            1004 => LLMError::AuthFailed(status_msg.to_string()),
            2013 => {
                if status_msg.contains("unknown model") {
                    LLMError::ModelNotFound(status_msg.to_string())
                } else {
                    LLMError::InvalidRequest(status_msg.to_string())
                }
            }
            _ => LLMError::ApiError(format!("MiniMax API error {}: {}", status_code, status_msg)),
        }
    }

    /// Extract visible content from a MiniMax message.
    /// Prefer `content`; if it's empty or pure whitespace, fall back to `reasoning_content`.
    fn extract_content(msg: &MiniMaxMessage) -> String {
        if !msg.content.trim().is_empty() {
            msg.content.trim().to_string()
        } else {
            msg.reasoning_content
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default()
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

        // Check MiniMax internal business error code
        if let Some(ref base_resp) = api_resp.base_resp {
            if base_resp.status_code != 0 {
                return Err(Self::map_base_resp_error(
                    base_resp.status_code,
                    &base_resp.status_msg,
                ));
            }
        }

        let msg = api_resp
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .map(|c| &c.message)
            .ok_or_else(|| LLMError::ApiError("no choices in MiniMax response".to_string()))?;

        let content = Self::extract_content(msg);

        let usage = api_resp.usage.as_ref();

        Ok(ChatResponse {
            content,
            model: api_resp.model,
            usage: Usage {
                prompt_tokens: usage.map(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: usage.map(|u| u.completion_tokens).unwrap_or(0),
                total_tokens: usage.map(|u| u.total_tokens).unwrap_or(0),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Fixture-based deserialization and content extraction tests ---

    #[test]
    fn test_simple_chat_deserialize_and_extract() {
        // simple-chat.json: content="", reasoning_content non-empty → extract reasoning_content
        let json = include_str!("../../tests/fixtures/llm/minimax/simple-chat.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
        let msg = &choice.message;
        let extracted = MiniMaxProvider::extract_content(msg);
        // The visible reply is in reasoning_content (content is empty)
        assert!(
            !extracted.is_empty(),
            "Expected non-empty extracted content from reasoning_content"
        );
        assert_eq!(extracted, msg.reasoning_content.as_ref().unwrap().trim());
    }

    #[test]
    fn test_m2_her_chat_deserialize_and_extract() {
        // m2-her-chat.json: content non-empty, no reasoning_content → extract content
        let json = include_str!("../../tests/fixtures/llm/minimax/m2-her-chat.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
        let msg = &choice.message;
        let extracted = MiniMaxProvider::extract_content(msg);
        assert!(
            !msg.content.trim().is_empty(),
            "m2-her fixture should have non-empty content"
        );
        assert_eq!(extracted, msg.content.trim());
        assert!(
            msg.reasoning_content.is_none() || msg.reasoning_content.as_ref().unwrap().is_empty()
        );
    }

    #[test]
    fn test_reasoning_heavy_deserialize_and_extract() {
        // reasoning-heavy.json: both content and reasoning_content non-empty → extract content
        let json = include_str!("../../tests/fixtures/llm/minimax/reasoning-heavy.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
        let msg = &choice.message;
        let extracted = MiniMaxProvider::extract_content(msg);
        assert!(
            !msg.content.trim().is_empty(),
            "reasoning-heavy fixture should have non-empty content"
        );
        // content takes priority when non-empty
        assert_eq!(extracted, msg.content.trim());
    }

    #[test]
    fn test_error_auth() {
        // error-auth.json: status_code=1004 → AuthFailed
        let json = include_str!("../../tests/fixtures/llm/minimax/error-auth.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        assert!(resp.base_resp.is_some());
        let base_resp = resp.base_resp.unwrap();
        assert_eq!(base_resp.status_code, 1004);
        let err =
            MiniMaxProvider::map_base_resp_error(base_resp.status_code, &base_resp.status_msg);
        matches!(err, LLMError::AuthFailed(msg) if msg.contains("login fail"));
    }

    #[test]
    fn test_error_invalid_model() {
        // error-invalid-model.json: status_code=2013 + "unknown model" → ModelNotFound
        let json = include_str!("../../tests/fixtures/llm/minimax/error-invalid-model.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        assert!(resp.base_resp.is_some());
        let base_resp = resp.base_resp.unwrap();
        assert_eq!(base_resp.status_code, 2013);
        let err =
            MiniMaxProvider::map_base_resp_error(base_resp.status_code, &base_resp.status_msg);
        matches!(
            err,
            LLMError::ModelNotFound(msg) if msg.contains("unknown model")
        );
    }

    #[test]
    fn test_error_empty_messages() {
        // error-empty-messages.json: status_code=2013 + "messages is empty" → InvalidRequest
        let json = include_str!("../../tests/fixtures/llm/minimax/error-empty-messages.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        assert!(resp.base_resp.is_some());
        let base_resp = resp.base_resp.unwrap();
        assert_eq!(base_resp.status_code, 2013);
        let err =
            MiniMaxProvider::map_base_resp_error(base_resp.status_code, &base_resp.status_msg);
        matches!(
            err,
            LLMError::InvalidRequest(msg) if msg.contains("messages is empty")
        );
    }

    #[test]
    fn test_error_missing_model() {
        // error-missing-model.json: status_code=2013 + "missing required parameter" → InvalidRequest
        let json = include_str!("../../tests/fixtures/llm/minimax/error-missing-model.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        assert!(resp.base_resp.is_some());
        let base_resp = resp.base_resp.unwrap();
        assert_eq!(base_resp.status_code, 2013);
        let err =
            MiniMaxProvider::map_base_resp_error(base_resp.status_code, &base_resp.status_msg);
        matches!(
            err,
            LLMError::InvalidRequest(msg) if msg.contains("missing required parameter")
        );
    }

    // --- Completion tokens details ---

    #[test]
    fn test_completion_tokens_details_present() {
        // simple-chat.json has completion_tokens_details with reasoning_tokens
        let json = include_str!("../../tests/fixtures/llm/minimax/simple-chat.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        let usage = resp.usage.unwrap();
        assert!(usage.completion_tokens_details.is_some());
        let details = usage.completion_tokens_details.unwrap();
        assert_eq!(details.reasoning_tokens, Some(50));
    }

    #[test]
    fn test_completion_tokens_details_absent() {
        // m2-her-chat.json does NOT have completion_tokens_details
        let json = include_str!("../../tests/fixtures/llm/minimax/m2-her-chat.json");
        let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
        let usage = resp.usage.unwrap();
        assert!(usage.completion_tokens_details.is_none());
    }
}
