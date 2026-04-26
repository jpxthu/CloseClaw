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
mod tests;
