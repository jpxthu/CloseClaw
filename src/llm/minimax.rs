//! MiniMax LLM Provider

#![allow(deprecated)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use crate::llm::ReqwestHttpClient;
use crate::llm::{ChatRequest, ChatResponse, HttpClient, LLMError, LLMProvider, ModelInfo, Usage};

#[path = "minimax_stream.rs"]
pub(crate) mod minimax_stream;

// ---------------------------------------------------------------------------//
// MiniMax /models API types                                                 //
// ---------------------------------------------------------------------------//

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

#[allow(dead_code)]
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
#[allow(dead_code)]
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

/// Response from GET /v1/models (MiniMax model list API, OpenAI-compatible)
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MiniMaxModelsResponse {
    data: Vec<MiniMaxModel>,
    #[serde(default)]
    object: String,
}

/// A single model entry from the /models API
#[allow(dead_code)]
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

// ---------------------------------------------------------------------------//
// Provider                                                                   //
// ---------------------------------------------------------------------------//

pub struct MiniMaxProvider {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) http_client: Arc<dyn HttpClient>,
}

impl MiniMaxProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, MINIMAX_API_URL.to_string())
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(std::env::var("MINIMAX_API_KEY").ok()?))
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
        let base = self
            .base_url
            .trim_end_matches("/chat/completions")
            .trim_end_matches("/v1");
        let url = format!("{}/v1/models", base);

        let req = reqwest::Request::new(
            reqwest::Method::GET,
            reqwest::Url::parse(&url).expect("invalid URL"),
        );
        let mut req = req;
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

        let api_resp: MiniMaxModelsResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

        let models: Vec<ModelInfo> = api_resp
            .data
            .into_iter()
            .map(|m| {
                use crate::llm::InputType;
                let model_id = m.id.clone();
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

        let api_resp: MiniMaxResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

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
#[path = "minimax/tests.rs"]
mod tests;
