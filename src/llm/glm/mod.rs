//! GLm LLM Provider

#![allow(deprecated)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use crate::llm::http_client::{HttpClient, ReqwestHttpClient};
use crate::llm::{
    ChatRequest, ChatResponse, LLMError, LLMProvider, ModelInfo, StreamingResponse, Usage,
};

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
// ---------------------------------------------------------------------------//
// GLM /models API types                                                     //
// ---------------------------------------------------------------------------//

/// Response from GET /api/coding/paas/v4/models (GLM model list API)
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct GlmModelsResponse {
    data: Vec<GlmModel>,
    #[serde(default)]
    object: Option<String>,
}

/// A single model entry from the /models API
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct GlmModel {
    id: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    created: Option<u64>,
    #[serde(default)]
    owned_by: Option<String>,
}

// ---------------------------------------------------------------------------//
// GLM Usage / Quota API types                                               //
// ---------------------------------------------------------------------------//

/// GLM quota API response wrapper.
#[derive(Debug, Deserialize)]
pub struct GlmQuotaResponse {
    pub code: u16,
    pub msg: String,
    pub data: GlmQuotaData,
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct GlmQuotaData {
    pub limits: Vec<GlmLimit>,
    #[serde(default)]
    pub level: String,
}

/// A single quota limit entry.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GlmLimit {
    #[serde(rename = "type")]
    limit_type: String,
    unit: u32,
    #[serde(default)]
    number: Option<u32>,
    #[serde(default)]
    usage: Option<u64>,
    #[serde(default)]
    remaining: Option<u64>,
    #[serde(default)]
    percentage: Option<u32>,
    #[serde(rename = "nextResetTime", default)]
    next_reset_time: Option<u64>,
    #[serde(rename = "usageDetails", default)]
    usage_details: Option<Vec<GlmUsageDetail>>,
}

/// Per-model usage breakdown.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GlmUsageDetail {
    #[serde(rename = "modelCode")]
    model_code: String,
    usage: u64,
}

pub struct GlmProvider {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) http_client: Arc<dyn HttpClient>,
}

impl GlmProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, GLM_API_URL.to_string())
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(std::env::var("GLM_API_KEY").ok()?))
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let http_client = Arc::new(ReqwestHttpClient::new().expect("Failed to create HTTP client"));
        Self {
            api_key,
            base_url,
            http_client,
        }
    }

    /// Create a provider with a custom `HttpClient` implementation.
    #[cfg(test)]
    pub(crate) fn with_http_client(
        api_key: String,
        base_url: String,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
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

    /// Fetch GLM account quota / usage info from the Usage API.
    ///
    /// The `base_url` should be the GLM API base (e.g. `https://open.bigmodel.cn/api`);
    /// this method appends `/paas/quota` internally.
    pub async fn fetch_usage(&self, base_url: &str) -> Result<GlmQuotaResponse, LLMError> {
        let url = format!("{}/paas/quota", base_url.trim_end_matches('/'));

        let mut req = reqwest::Request::new(
            reqwest::Method::GET,
            reqwest::Url::parse(&url).expect("invalid URL"),
        );
        req.headers_mut().insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", self.api_key).parse().unwrap(),
        );

        let response = self
            .http_client
            .execute(req)
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let quota: GlmQuotaResponse = response.json().await.map_err(|e| {
            LLMError::ApiError(format!("failed to parse GLM quota response: {}", e))
        })?;

        if !quota.success {
            return Err(LLMError::ApiError(format!(
                "GLM quota API error {}: {}",
                quota.code, quota.msg
            )));
        }

        Ok(quota)
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
    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        // GLM /models endpoint is at /api/paas/v4/models (not /coding/paas/v4!)
        // base_url is the chat endpoint (e.g. https://open.bigmodel.cn/api/coding/paas/v4/chat/completions);
        // Replace /coding/paas/v4/chat/completions with /paas/v4/models
        let models_url = self
            .base_url
            .replace("/coding/paas/v4/chat/completions", "/paas/v4/models")
            .replace("/chat/completions", "/models");

        let mut req = reqwest::Request::new(
            reqwest::Method::GET,
            reqwest::Url::parse(&models_url).expect("invalid URL"),
        );
        req.headers_mut().insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", bearer_token).parse().unwrap(),
        );

        let response = match timeout(Duration::from_secs(10), self.http_client.execute(req)).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(LLMError::NetworkError(e.to_string())),
            Err(_) => {
                return Err(LLMError::NetworkError(
                    "fetch_model_list timed out after 10s".to_string(),
                ))
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let api_resp: GlmModelsResponse = response.json().await.map_err(|e| {
            LLMError::ApiError(format!("failed to parse GLM /models response: {}", e))
        })?;

        let models: Vec<ModelInfo> = api_resp
            .data
            .into_iter()
            .map(|m| {
                use crate::llm::InputType;
                let model_id = m.id.clone();
                // Look up knowledge base for metadata; safe defaults if not found.
                let kb = crate::llm::ProviderModelKnowledge::new();
                let params = kb.find("glm", &model_id);
                let (context_window, max_tokens, default_temperature, reasoning, input_types) =
                    match params {
                        Some(p) => (
                            p.context_window,
                            p.max_tokens,
                            Some(p.default_temperature),
                            p.reasoning,
                            p.input_types,
                        ),
                        None => (128_000, 8_192, Some(0.7), false, vec![InputType::Text]),
                    };
                let name = format!(
                    "GLM {}",
                    model_id
                        .trim_start_matches("glm-")
                        .trim_start_matches("GLM-")
                );
                ModelInfo {
                    id: model_id,
                    name,
                    context_window,
                    max_tokens,
                    default_temperature,
                    reasoning,
                    input_types,
                }
            })
            .collect();

        Ok(models)
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let req_body = GlmRequest {
            model: &request.model,
            messages: &request.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let mut req = reqwest::Request::new(
            reqwest::Method::POST,
            reqwest::Url::parse(&self.base_url).expect("invalid URL"),
        );
        {
            let headers = req.headers_mut();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key).parse().unwrap(),
            );
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                "application/json".parse().unwrap(),
            );
            *req.body_mut() = Some(serde_json::to_string(&req_body).unwrap().into());
        }

        let response = self
            .http_client
            .execute(req)
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
