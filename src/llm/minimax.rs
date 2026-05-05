//! MiniMax LLM Provider

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, ModelInfo, Usage};

#[path = "minimax_stream.rs"]
pub(crate) mod minimax_stream;

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

// ---------------------------------------------------------------------------//
// MiniMax /models API types                                                 //
// ---------------------------------------------------------------------------//

/// Response from GET /v1/models (MiniMax model list API, OpenAI-compatible)
#[derive(Debug, Deserialize)]
struct MiniMaxModelsResponse {
    data: Vec<MiniMaxModel>,
    #[serde(default)]
    object: String,
}

/// A single model entry from the /models API
#[derive(Debug, Deserialize)]
struct MiniMaxModel {
    id: String,
    #[serde(default)]
    object: String,
    #[serde(default)]
    created: u64,
    #[serde(default)]
    owned_by: String,
}

pub struct MiniMaxProvider {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) http_client: Client,
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

    pub(crate) fn map_status_error(status: reqwest::StatusCode, body: String) -> LLMError {
        match status.as_u16() {
            401 | 403 => LLMError::AuthFailed(body),
            404 => LLMError::ModelNotFound(body),
            422 => LLMError::InvalidRequest(body),
            429 => LLMError::RateLimitExceeded,
            _ => LLMError::ApiError(format!("unexpected status {}: {}", status, body)),
        }
    }

    /// Map MiniMax internal status_code to LLMError
    pub(crate) fn map_base_resp_error(status_code: i32, status_msg: &str) -> LLMError {
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

    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        // MiniMax /v1/models uses OpenAI-compatible format.
        // The base_url is the chat endpoint; strip /chat/completions if present.
        let base = self
            .base_url
            .trim_end_matches("/chat/completions")
            .trim_end_matches("/v1");
        let url = format!("{}/v1/models", base);

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
            return Err(Self::map_status_error(status, body));
        }

        let api_resp: MiniMaxModelsResponse = response.json().await.map_err(|e| {
            LLMError::ApiError(format!("failed to parse MiniMax /models response: {}", e))
        })?;

        let models: Vec<ModelInfo> = api_resp
            .data
            .into_iter()
            .map(|m| {
                use crate::llm::InputType;
                let model_id = m.id.clone();
                // Look up knowledge base for metadata; safe defaults if not found.
                let kb = crate::llm::ProviderModelKnowledge::new();
                let params = kb.find("minimax", &model_id);
                let (context_window, max_tokens, default_temperature, reasoning, input_types) =
                    match params {
                        Some(p) => (
                            p.context_window,
                            p.max_tokens,
                            Some(p.default_temperature),
                            p.reasoning,
                            p.input_types,
                        ),
                        None => (32_768, 8_192, Some(0.7), false, vec![InputType::Text]),
                    };
                ModelInfo {
                    id: model_id.clone(),
                    name: format!("MiniMax {}", model_id.trim_start_matches("MiniMax-")),
                    context_window,
                    max_tokens,
                    default_temperature,
                    // reasoning: we cannot determine from /models alone, use KB default
                    reasoning,
                    input_types,
                }
            })
            .collect();

        Ok(models)
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

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<crate::llm::StreamingResponse, LLMError> {
        minimax_stream::send_streaming_request(self, request).await
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
