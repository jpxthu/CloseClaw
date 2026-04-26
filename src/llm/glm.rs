//! GLm LLM Provider

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, StreamingResponse, Usage};

/// GLM API endpoint
const GLM_API_URL: &str = "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions";

/// GLM API request body
#[derive(Debug, Serialize)]
struct GlmRequest<'a> {
    model: &'a str,
    messages: &'a [crate::llm::Message],
    temperature: f32,
    max_tokens: Option<u32>,
}

/// GLM API response body
#[derive(Debug, Deserialize)]
struct GlmResponse {
    #[serde(default)]
    choices: Option<Vec<GlmChoice>>,
    #[serde(default)]
    usage: Option<GlmUsage>,
    #[serde(default)]
    model: String,
    /// Top-level GLM error (e.g. code="1211", "1214")
    #[serde(default)]
    error: Option<GlmErrorBody>,
}

#[derive(Debug, Deserialize)]
struct GlmChoice {
    message: GlmMessage,
}

#[derive(Debug, Deserialize)]
struct GlmMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
    /// GLM reasoning content for reasoning models (glm-5.1, glm-4.7, etc.).
    /// When content is empty, the visible reply is in this field.
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
    #[serde(default)]
    completion_tokens_details: Option<GlmCompletionTokensDetails>,
    #[serde(default)]
    prompt_tokens_details: Option<GlmPromptTokensDetails>,
}

/// GLM completion tokens details
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmCompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u32>,
}

/// GLM prompt tokens details
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmPromptTokensDetails {
    /// Cached tokens from context caching / system prompt optimization.
    #[serde(default)]
    cached_tokens: Option<u32>,
}

/// GLM top-level error body
#[derive(Debug, Deserialize)]
struct GlmErrorBody {
    code: String,
    message: String,
}

pub struct GlmProvider {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) http_client: Client,
}

impl GlmProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, GLM_API_URL.to_string())
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(std::env::var("GLM_API_KEY").ok()?))
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

    pub(crate) fn map_status_error(status: reqwest::StatusCode, body: String) -> LLMError {
        match status.as_u16() {
            401 | 403 => LLMError::AuthFailed(body),
            404 => LLMError::ModelNotFound(body),
            422 => LLMError::InvalidRequest(body),
            429 => LLMError::RateLimitExceeded,
            _ => LLMError::ApiError(format!("unexpected status {}: {}", status, body)),
        }
    }

    /// Map GLM error code to LLMError.
    ///
    /// - "1211" → ModelNotFound (model does not exist)
    /// - "1214" → InvalidRequest (e.g. empty messages)
    /// - others → ApiError
    pub(crate) fn map_glm_error(code: &str, message: &str) -> LLMError {
        match code {
            "1211" => LLMError::ModelNotFound(message.to_string()),
            "1214" => LLMError::InvalidRequest(message.to_string()),
            _ => LLMError::ApiError(format!("GLM API error {}: {}", code, message)),
        }
    }

    /// Extract visible content from a GLM message.
    /// Prefer `content`; if it's empty or pure whitespace, fall back to `reasoning_content`.
    fn extract_content(msg: &GlmMessage) -> String {
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

    fn parse_chat_response(api_resp: GlmResponse) -> Result<ChatResponse, LLMError> {
        if let Some(ref err) = api_resp.error {
            return Err(Self::map_glm_error(&err.code, &err.message));
        }
        let msg = api_resp
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .map(|c| &c.message)
            .ok_or_else(|| LLMError::ApiError("no choices in GLM response".to_string()))?;
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

#[async_trait]
impl LLMProvider for GlmProvider {
    fn name(&self) -> &str {
        "glm"
    }

    fn models(&self) -> Vec<&str> {
        vec![
            "glm-5.1",
            "glm-4.7",
            "glm-4.5-air",
            "GLM-4.5-Air",
            "GLM-4.7",
            "glm-5-turbo",
        ]
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let req_body = GlmRequest {
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

        let api_resp: GlmResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(format!("failed to parse GLM response: {}", e)))?;

        Self::parse_chat_response(api_resp)
    }

    async fn chat_streaming(&self, request: ChatRequest) -> Result<StreamingResponse, LLMError> {
        crate::llm::glm_stream::send_streaming_request(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Fixture-based deserialization and content extraction tests ---

    #[test]
    fn test_glm_5_1_chat_extract_reasoning() {
        // glm-5.1-chat.json: content empty → extract reasoning_content
        let json = include_str!("../../tests/fixtures/llm/glm/glm-5.1-chat.json");
        let resp: GlmResponse = serde_json::from_str(json).unwrap();
        let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
        let msg = &choice.message;
        let extracted = GlmProvider::extract_content(msg);
        assert!(
            !msg.content.trim().is_empty() == false,
            "content should be empty/whitespace in this fixture"
        );
        assert!(
            msg.reasoning_content.is_some() && !msg.reasoning_content.as_ref().unwrap().is_empty(),
            "reasoning_content should be non-empty in this fixture"
        );
        assert!(
            !extracted.is_empty(),
            "should extract from reasoning_content"
        );
        assert_eq!(extracted, msg.reasoning_content.as_ref().unwrap().trim());
        // Verify usage fields
        let usage = resp.usage.as_ref().unwrap();
        assert_eq!(usage.completion_tokens, 30);
        assert_eq!(usage.prompt_tokens, 11);
        assert_eq!(usage.total_tokens, 41);
        let details = usage.completion_tokens_details.as_ref().unwrap();
        assert_eq!(details.reasoning_tokens, Some(30));
        let prompt_details = usage.prompt_tokens_details.as_ref().unwrap();
        assert_eq!(prompt_details.cached_tokens, Some(0));
    }

    #[test]
    fn test_glm_4_7_simple_chat_extract_reasoning() {
        // glm-4.7-simple-chat.json: content empty → extract reasoning_content
        let json = include_str!("../../tests/fixtures/llm/glm/glm-4.7-simple-chat.json");
        let resp: GlmResponse = serde_json::from_str(json).unwrap();
        let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
        let msg = &choice.message;
        let extracted = GlmProvider::extract_content(msg);
        assert!(msg.content.trim().is_empty(), "content should be empty");
        assert!(
            msg.reasoning_content.is_some() && !msg.reasoning_content.as_ref().unwrap().is_empty(),
            "reasoning_content should be non-empty"
        );
        assert!(
            !extracted.is_empty(),
            "should extract from reasoning_content"
        );
        assert_eq!(extracted, msg.reasoning_content.as_ref().unwrap().trim());
        // GLM-4.7 model name
        assert_eq!(resp.model, "GLM-4.7");
        // Verify cached_tokens in prompt_tokens_details
        let usage = resp.usage.as_ref().unwrap();
        let prompt_details = usage.prompt_tokens_details.as_ref().unwrap();
        assert_eq!(prompt_details.cached_tokens, Some(10));
    }

    #[test]
    fn test_glm_4_5_air_chat_extract_reasoning() {
        // glm-4.5-air-chat.json: AIR model, content empty → extract reasoning_content
        let json = include_str!("../../tests/fixtures/llm/glm/glm-4.5-air-chat.json");
        let resp: GlmResponse = serde_json::from_str(json).unwrap();
        let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
        let msg = &choice.message;
        let extracted = GlmProvider::extract_content(msg);
        assert!(msg.content.trim().is_empty(), "content should be empty");
        assert!(
            msg.reasoning_content.is_some() && !msg.reasoning_content.as_ref().unwrap().is_empty(),
            "reasoning_content should be non-empty"
        );
        assert!(
            !extracted.is_empty(),
            "should extract from reasoning_content"
        );
        assert_eq!(resp.model, "GLM-4.5-Air");
    }

    #[test]
    fn test_glm_5_1_multi_turn() {
        // glm-5.1-multi-turn.json: multi-turn conversation parsing
        let json = include_str!("../../tests/fixtures/llm/glm/glm-5.1-multi-turn.json");
        let resp: GlmResponse = serde_json::from_str(json).unwrap();
        let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
        let msg = &choice.message;
        let extracted = GlmProvider::extract_content(msg);
        assert!(
            !extracted.is_empty(),
            "multi-turn should extract reasoning_content"
        );
        assert_eq!(resp.model, "glm-5.1");
        let usage = resp.usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, 20);
        assert_eq!(usage.completion_tokens, 30);
        assert_eq!(usage.total_tokens, 50);
    }

    // --- Error mapping tests ---

    #[test]
    fn test_glm_error_invalid_model() {
        // glm-error-invalid-model.json: code="1211" → ModelNotFound
        let json = include_str!("../../tests/fixtures/llm/glm/glm-error-invalid-model.json");
        let resp: GlmResponse = serde_json::from_str(json).unwrap();
        let err_body = resp.error.as_ref().unwrap();
        assert_eq!(err_body.code, "1211");
        let err = GlmProvider::map_glm_error(&err_body.code, &err_body.message);
        matches!(err, LLMError::ModelNotFound(msg) if msg.contains("模型不存在"));
    }

    #[test]
    fn test_glm_error_empty_messages() {
        // glm-error-empty-messages.json: code="1214" → InvalidRequest
        let json = include_str!("../../tests/fixtures/llm/glm/glm-error-empty-messages.json");
        let resp: GlmResponse = serde_json::from_str(json).unwrap();
        let err_body = resp.error.as_ref().unwrap();
        assert_eq!(err_body.code, "1214");
        let err = GlmProvider::map_glm_error(&err_body.code, &err_body.message);
        matches!(err, LLMError::InvalidRequest(msg) if msg.contains("输入不能为空"));
    }

    #[test]
    fn test_glm_error_unknown_code() {
        // Unknown code maps to ApiError
        let err = GlmProvider::map_glm_error("9999", "some unknown error");
        matches!(err, LLMError::ApiError(msg) if msg.contains("9999"));
    }

    // --- extract_content edge cases ---

    #[test]
    fn test_extract_content_prefers_non_empty_content() {
        // When content is non-empty, reasoning_content should be ignored
        let msg = GlmMessage {
            role: "assistant".to_string(),
            content: "Hello, World!".to_string(),
            reasoning_content: Some("I am thinking...".to_string()),
        };
        let extracted = GlmProvider::extract_content(&msg);
        assert_eq!(extracted, "Hello, World!");
    }

    #[test]
    fn test_extract_content_falls_back_to_reasoning_when_content_empty() {
        // content empty/whitespace → fall back to reasoning_content
        let msg = GlmMessage {
            role: "assistant".to_string(),
            content: "   ".to_string(),
            reasoning_content: Some("Thinking process...".to_string()),
        };
        let extracted = GlmProvider::extract_content(&msg);
        assert_eq!(extracted, "Thinking process...");
    }

    #[test]
    fn test_extract_content_whitespace_only_reasoning() {
        // content empty, reasoning_content whitespace-only → returns empty
        let msg = GlmMessage {
            role: "assistant".to_string(),
            content: "".to_string(),
            reasoning_content: Some("   ".to_string()),
        };
        let extracted = GlmProvider::extract_content(&msg);
        assert_eq!(extracted, "");
    }

    #[test]
    fn test_extract_content_both_empty() {
        // content empty, no reasoning_content → returns empty
        let msg = GlmMessage {
            role: "assistant".to_string(),
            content: "".to_string(),
            reasoning_content: None,
        };
        let extracted = GlmProvider::extract_content(&msg);
        assert_eq!(extracted, "");
    }

    // --- Token details deserialization ---

    #[test]
    fn test_glm_5_1_reasoning_tokens_details() {
        // glm-5.1-reasoning.json: verify completion_tokens_details and prompt_tokens_details
        let json = include_str!("../../tests/fixtures/llm/glm/glm-5.1-reasoning.json");
        let resp: GlmResponse = serde_json::from_str(json).unwrap();
        let usage = resp.usage.as_ref().unwrap();
        assert_eq!(usage.completion_tokens, 200);
        assert_eq!(usage.prompt_tokens, 17);
        let details = usage.completion_tokens_details.as_ref().unwrap();
        assert_eq!(details.reasoning_tokens, Some(200));
        let prompt_details = usage.prompt_tokens_details.as_ref().unwrap();
        assert_eq!(prompt_details.cached_tokens, Some(0));
    }
}
