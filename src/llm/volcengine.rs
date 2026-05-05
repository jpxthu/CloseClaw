//! VolcEngine (火山引擎) LLM Provider
//!
//! Uses the Volcano Ark (方舟) API. Chat endpoint is OpenAI-compatible at
//! `base_url/chat/completions`. Model list is fetched from `base_url/models`.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, ModelInfo, Usage};

/// VolcEngine API endpoint
const VOLCENGINE_API_URL: &str = "https://ARK.cn-beijing.volces.com/api/v3/chat/completions";

/// VolcEngine chat request body (OpenAI-compatible)
#[derive(Debug, Serialize)]
struct VolcEngineRequest<'a> {
    model: &'a str,
    messages: &'a [crate::llm::Message],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

/// VolcEngine chat response body (OpenAI-compatible)
#[derive(Debug, Deserialize)]
struct VolcEngineResponse {
    id: Option<String>,
    model: String,
    choices: Vec<VolcEngineChoice>,
    usage: Option<VolcEngineUsage>,
    /// VolcEngine error object (e.g. code, message)
    #[serde(default)]
    error: Option<VolcEngineErrorBody>,
}

#[derive(Debug, Deserialize)]
struct VolcEngineChoice {
    message: VolcEngineMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VolcEngineMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Default)]
struct VolcEngineUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

/// VolcEngine error body (returned inside response JSON on business errors)
#[derive(Debug, Deserialize)]
struct VolcEngineErrorBody {
    code: Option<String>,
    message: Option<String>,
}

// ---------------------------------------------------------------------------//
// VolcEngine /models API types (方舟模型列表 API)                            //
// ---------------------------------------------------------------------------//

/// Response from GET base_url/models (Volcano Ark model list API)
#[derive(Debug, Deserialize)]
struct VolcEngineModelsResponse {
    data: Vec<VolcEngineModel>,
}

/// A single model entry from the /models API
#[derive(Debug, Deserialize)]
struct VolcEngineModel {
    /// Model identifier string (e.g. "doubao-1.5-pro")
    model_id: String,
    /// Model display name
    model_name: Option<String>,
    /// Model status: "Online", "Creating", "Shutdown", "Retiring", etc.
    #[serde(default)]
    status: String,
    /// Model domain classification — we filter for domain == "LLM"
    #[serde(default)]
    domain: String,
    /// Token limits and other metadata
    #[serde(default)]
    properties: VolcEngineModelProperties,
}

#[derive(Debug, Deserialize, Default)]
struct VolcEngineModelProperties {
    /// Context window size in tokens
    #[serde(default)]
    context_window: Option<u32>,
    /// Maximum output tokens
    #[serde(default)]
    max_tokens: Option<u32>,
    /// Default sampling temperature
    #[serde(default)]
    temperature: Option<f32>,
    /// Supported input modalities
    #[serde(default)]
    input_modalities: Vec<String>,
}

// ---------------------------------------------------------------------------//
// Provider implementation                                                    //
// ---------------------------------------------------------------------------//

pub struct VolcEngineProvider {
    api_key: String,
    base_url: String,
    http_client: Client,
}

impl VolcEngineProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, VOLCENGINE_API_URL.to_string())
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

    /// Extract visible content from a VolcEngine message.
    /// VolcEngine does not use reasoning_content; we simply trim whitespace.
    fn extract_content(msg: &VolcEngineMessage) -> String {
        msg.content.trim().to_string()
    }

    fn models_static() -> Vec<&'static str> {
        // Models hardcoded for the `models()` method (non-discovery path)
        vec![
            "doubao-1.5-pro",
            "doubao-1.5-lite",
            "doubao-1.5-pro-32k",
            "doubao-1.5-lite-32k",
        ]
    }
}

#[async_trait]
impl LLMProvider for VolcEngineProvider {
    fn name(&self) -> &str {
        "volcengine"
    }

    fn models(&self) -> Vec<&str> {
        Self::models_static()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let req_body = VolcEngineRequest {
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

        let api_resp: VolcEngineResponse = response.json().await.map_err(|e| {
            LLMError::ApiError(format!("failed to parse VolcEngine response: {}", e))
        })?;

        // Check for business-level error in response body
        if let Some(ref err) = api_resp.error {
            let code = err.code.as_deref().unwrap_or("");
            let msg = err.message.as_deref().unwrap_or("unknown error");
            return Err(match code {
                "1001" | "1002" => LLMError::AuthFailed(msg.to_string()),
                "1103" => LLMError::ModelNotFound(msg.to_string()),
                "1111" | "1112" => LLMError::InvalidRequest(msg.to_string()),
                "2001" => LLMError::RateLimitExceeded,
                _ => LLMError::ApiError(format!("VolcEngine API error {}: {}", code, msg)),
            });
        }

        let msg = api_resp
            .choices
            .first()
            .map(|c| &c.message)
            .ok_or_else(|| LLMError::ApiError("no choices in VolcEngine response".to_string()))?;

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
        "VolcEngine"
    }

    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bearer_token))
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }

        let api_resp: VolcEngineModelsResponse = response.json().await.map_err(|e| {
            LLMError::ApiError(format!(
                "failed to parse VolcEngine /models response: {}",
                e
            ))
        })?;

        let models: Vec<ModelInfo> = api_resp
            .data
            .into_iter()
            // Filter: domain must be "LLM"; status must NOT be Shutdown or Retiring
            .filter(|m| {
                m.domain.eq_ignore_ascii_case("LLM")
                    && !m.status.eq_ignore_ascii_case("Shutdown")
                    && !m.status.eq_ignore_ascii_case("Retiring")
            })
            .map(|m| {
                let model_id = m.model_id.clone();
                let props = &m.properties;
                let input_types: Vec<crate::llm::InputType> = props
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

                ModelInfo {
                    id: model_id.clone(),
                    name: m.model_name.unwrap_or_else(|| model_id.clone()),
                    context_window: props.context_window.unwrap_or(32_768),
                    max_tokens: props.max_tokens.unwrap_or(4_096),
                    default_temperature: props.temperature,
                    // VolcEngine does not expose a reasoning flag directly;
                    // we conservatively set reasoning=false here.
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
