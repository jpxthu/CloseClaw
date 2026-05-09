//! DeepSeek LLM Provider
//!
//! Uses the DeepSeek API. Chat endpoint is OpenAI-compatible at
//! `base_url/chat/completions`. Model list is fetched from `base_url/models`.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, ModelInfo, Usage};

/// DeepSeek API endpoint
const DEEPSEEK_API_URL: &str = "https://api.deepseek.com";

/// DeepSeek chat request body (OpenAI-compatible)
#[derive(Debug, Serialize)]
struct DeepSeekRequest<'a> {
    model: &'a str,
    messages: &'a [crate::llm::Message],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

/// DeepSeek chat response body (OpenAI-compatible)
#[derive(Debug, Deserialize)]
struct DeepSeekResponse {
    id: Option<String>,
    model: String,
    choices: Vec<DeepSeekChoice>,
    usage: Option<DeepSeekUsage>,
    /// DeepSeek error object (e.g. code, message)
    #[serde(default)]
    error: Option<DeepSeekErrorBody>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    message: DeepSeekMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekMessage {
    role: String,
    content: String,
    /// DeepSeek reasoning content for reasoning models (deepseek-v4-pro, etc.).
    /// When content is empty, the visible reply is in this field.
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct DeepSeekUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

/// DeepSeek error body (returned inside response JSON on business errors)
#[derive(Debug, Deserialize)]
struct DeepSeekErrorBody {
    code: Option<String>,
    message: Option<String>,
}

// ---------------------------------------------------------------------------//
// DeepSeek /models API types                                                //
// ---------------------------------------------------------------------------//

/// Response from GET base_url/models (OpenAI-compatible model list API)
#[derive(Debug, Deserialize)]
struct DeepSeekModelsResponse {
    data: Vec<DeepSeekModel>,
}

/// A single model entry from the /models API
#[derive(Debug, Deserialize)]
struct DeepSeekModel {
    id: String,
    /// Human-readable display name
    #[serde(default)]
    display_name: Option<String>,
    /// Model status: "online", "deprecated", etc.
    #[serde(default)]
    status: Option<String>,
    /// Model context window size in tokens
    #[serde(default)]
    context_window: Option<u32>,
    /// Maximum output tokens
    #[serde(default)]
    max_output_tokens: Option<u32>,
    /// Supported input modalities
    #[serde(default)]
    input_modalities: Vec<String>,
    /// Supported output modalities
    #[serde(default)]
    output_modalities: Vec<String>,
    /// Pricing information
    #[serde(default)]
    pricing: Option<DeepSeekModelPricing>,
}

#[derive(Debug, Deserialize, Default)]
struct DeepSeekModelPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

// ---------------------------------------------------------------------------//
// Provider implementation                                                    //
// ---------------------------------------------------------------------------//

pub struct DeepSeekProvider {
    api_key: String,
    base_url: String,
    http_client: Client,
}

impl DeepSeekProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, DEEPSEEK_API_URL.to_string())
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

    fn map_http_error(status: reqwest::StatusCode, body: &str) -> LLMError {
        match status.as_u16() {
            401 | 403 => LLMError::AuthFailed(body.to_string()),
            404 => LLMError::ModelNotFound(body.to_string()),
            422 => LLMError::InvalidRequest(body.to_string()),
            429 => LLMError::RateLimitExceeded,
            _ => LLMError::ApiError(format!("unexpected status {}: {}", status, body)),
        }
    }

    /// Extract visible content from a DeepSeek message.
    /// Prefer `content`; if it's empty or pure whitespace, fall back to `reasoning_content`.
    fn extract_content(msg: &DeepSeekMessage) -> String {
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

    fn models_static() -> Vec<&'static str> {
        vec!["deepseek-v4-flash", "deepseek-v4-pro"]
    }
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    fn name(&self) -> &str {
        "deepseek"
    }

    fn models(&self) -> Vec<&str> {
        Self::models_static()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let req_body = DeepSeekRequest {
            model: &request.model,
            messages: &request.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }

        let api_resp: DeepSeekResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(format!("failed to parse DeepSeek response: {}", e)))?;

        // Check for business-level error in response body
        if let Some(ref err) = api_resp.error {
            let code = err.code.as_deref().unwrap_or("");
            let msg = err.message.as_deref().unwrap_or("unknown error");
            return Err(match code {
                "invalid_api_key" | "forbidden" => LLMError::AuthFailed(msg.to_string()),
                "model_not_found" | "1301" => LLMError::ModelNotFound(msg.to_string()),
                "invalid_request" | "1302" => LLMError::InvalidRequest(msg.to_string()),
                "rate_limit_exceeded" | "1401" => LLMError::RateLimitExceeded,
                _ => LLMError::ApiError(format!("DeepSeek API error {}: {}", code, msg)),
            });
        }

        let msg = api_resp
            .choices
            .first()
            .map(|c| &c.message)
            .ok_or_else(|| LLMError::ApiError("no choices in DeepSeek response".to_string()))?;

        let content = Self::extract_content(msg);
        let usage = api_resp.usage.as_ref();

        Ok(ChatResponse {
            content,
            model: api_resp.model,
            usage: Usage {
                prompt_tokens: usage.and_then(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: usage.and_then(|u| u.completion_tokens).unwrap_or(0),
                total_tokens: usage.and_then(|u| u.total_tokens).unwrap_or(0),
            },
        })
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<crate::llm::StreamingResponse, LLMError> {
        // Default implementation: call chat() and wrap as single-chunk stream.
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let response = self.chat(request).await?;
        let _ = tx
            .send(crate::llm::ChatStreamChunk::Text(response.content))
            .await;
        let _ = tx
            .send(crate::llm::ChatStreamChunk::Done {
                model: response.model,
                usage: response.usage,
            })
            .await;
        Ok(rx)
    }

    fn provider_display_name(&self) -> &str {
        "DeepSeek"
    }

    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bearer_token))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }

        let api_resp: DeepSeekModelsResponse = response.json().await.map_err(|e| {
            LLMError::ApiError(format!("failed to parse DeepSeek /models response: {}", e))
        })?;

        let models: Vec<ModelInfo> = api_resp
            .data
            .into_iter()
            // Filter: only models that are not deprecated/shutdown
            .filter(|m| {
                m.status
                    .as_ref()
                    .map(|s| {
                        !s.eq_ignore_ascii_case("deprecated") && !s.eq_ignore_ascii_case("shutdown")
                    })
                    .unwrap_or(true)
            })
            .map(|m| {
                let input_types: Vec<crate::llm::InputType> = m
                    .input_modalities
                    .iter()
                    .filter_map(|m| match m.to_lowercase().as_str() {
                        "image" => Some(crate::llm::InputType::Image),
                        _ => Some(crate::llm::InputType::Text),
                    })
                    .collect();
                let input_types = if input_types.is_empty() {
                    vec![crate::llm::InputType::Text]
                } else {
                    input_types
                };

                // DeepSeek does not expose a reasoning flag in /models response.
                // The reasoning_content field in chat responses indicates a reasoning model,
                // but we cannot determine this from /models alone. Conservatively set reasoning=false.
                ModelInfo {
                    id: m.id.clone(),
                    name: m.display_name.clone().unwrap_or_else(|| m.id.clone()),
                    context_window: m.context_window.unwrap_or(64_000),
                    max_tokens: m.max_output_tokens.unwrap_or(8_192),
                    default_temperature: None,
                    reasoning: false,
                    input_types,
                }
            })
            .collect();

        Ok(models)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
