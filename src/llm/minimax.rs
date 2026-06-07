//! MiniMax LLM Provider — pure HTTP transport for the
//! MiniMax Chat Completions API.

use crate::llm::provider::{Provider, ProviderError, Result, SseStream};
use crate::llm::types::{InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawUsage};
use crate::llm::{LLMError, ModelInfo, ModelLister};
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use serde::Deserialize;
use std::sync::OnceLock;

#[path = "minimax_stream.rs"]
pub(crate) mod minimax_stream;

// ---------------------------------------------------------------------------//
// Constants                                                                  //
// ---------------------------------------------------------------------------//

const MINIMAX_API_URL: &str = "https://api.minimax.chat/v1/chat/completions";

// ---------------------------------------------------------------------------//
// Request / Response types                                                    //
// ---------------------------------------------------------------------------//

/// MiniMax API response body
#[derive(Debug, Deserialize)]
pub(crate) struct MiniMaxResponse {
    #[serde(default)]
    choices: Option<Vec<MiniMaxChoice>>,
    #[serde(default)]
    usage: Option<MiniMaxUsage>,
    #[serde(default)]
    #[allow(dead_code)]
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

// ---------------------------------------------------------------------------//
// Provider struct                                                             //
// ---------------------------------------------------------------------------//

pub struct MiniMaxProvider {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) client: Client,
    supported_protocols: Vec<ProtocolId>,
}

impl MiniMaxProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, MINIMAX_API_URL.to_string())
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(std::env::var("MINIMAX_API_KEY").ok()?))
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            base_url,
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("anthropic")],
        }
    }

    /// Create a provider with a custom `reqwest::Client`.
    #[cfg(test)]
    pub(crate) fn with_http_client(api_key: String, base_url: String, client: Client) -> Self {
        Self {
            api_key,
            base_url,
            client,
            supported_protocols: vec![ProtocolId::new("anthropic")],
        }
    }

    // ── Error mapping (Provider) ────────────────────────────────────────

    /// Map HTTP status error to ProviderError.
    pub(crate) fn map_status_error(status: reqwest::StatusCode, body: String) -> ProviderError {
        ProviderError::Legacy(format!("MiniMax API error {}: {}", status, body))
    }

    /// Map MiniMax internal base_resp status_code to ProviderError.
    pub(crate) fn map_base_resp_error(status_code: i32, status_msg: &str) -> ProviderError {
        ProviderError::Legacy(format!(
            "MiniMax business error {}: {}",
            status_code, status_msg
        ))
    }

    // ── Content extraction ──────────────────────────────────────────────

    /// Extract visible content from a MiniMax message.
    /// Prefer `content`; if empty/whitespace, fall back to
    /// `reasoning_content`.
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

    // ── Response parsing (Provider) ─────────────────────────────────────

    /// Parse a MiniMax chat response into InternalResponse.
    pub(crate) fn parse_chat_response(api_resp: MiniMaxResponse) -> Result<InternalResponse> {
        // Check base_resp business errors
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
            .ok_or_else(|| ProviderError::Legacy("no choices in MiniMax response".to_string()))?;

        let content = Self::extract_content(msg);

        // Build content blocks: Thinking from
        // reasoning_content, Text from content
        let mut content_blocks = Vec::new();
        if let Some(ref rc) = msg.reasoning_content {
            if !rc.trim().is_empty() {
                content_blocks.push(RawContentBlock::Thinking(rc.trim().to_string()));
            }
        }
        if !content.is_empty() {
            content_blocks.push(RawContentBlock::Text(content));
        }

        let usage = api_resp.usage.as_ref();
        Ok(InternalResponse {
            content_blocks,
            usage: RawUsage {
                prompt_tokens: usage.map(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: usage.map(|u| u.completion_tokens).unwrap_or(0),
                total_tokens: usage.map(|u| u.total_tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }
}

// ---------------------------------------------------------------------------//
// Provider trait implementation                                               //
// ---------------------------------------------------------------------------//

#[async_trait]
impl Provider for MiniMaxProvider {
    fn id(&self) -> &str {
        "minimax"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn supported_protocols(&self) -> &[ProtocolId] {
        &self.supported_protocols
    }

    fn http_client(&self) -> &Client {
        &self.client
    }

    fn default_headers(&self) -> &HeaderMap {
        static EMPTY: OnceLock<HeaderMap> = OnceLock::new();
        EMPTY.get_or_init(HeaderMap::new)
    }

    async fn send(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<InternalResponse> {
        let response = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let api_resp: MiniMaxResponse = response.json().await.map_err(ProviderError::Reqwest)?;

        Self::parse_chat_response(api_resp)
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream> {
        minimax_stream::send_streaming_request(self, body).await
    }
}

// ---------------------------------------------------------------------------//
// ModelLister (kept for config_wizard; to be removed when migrated)           //
// ---------------------------------------------------------------------------//

#[async_trait]
impl ModelLister for MiniMaxProvider {
    async fn fetch_model_list(
        &self,
        bearer_token: &str,
    ) -> std::result::Result<Vec<ModelInfo>, LLMError> {
        let base = self
            .base_url
            .trim_end_matches("/chat/completions")
            .trim_end_matches("/v1");
        let url = format!("{}/v1/models", base);

        let response = match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.client
                .get(&url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", bearer_token),
                )
                .send(),
        )
        .await
        {
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
            return Err(LLMError::ApiError(format!(
                "MiniMax API error {}: {}",
                status, body
            )));
        }

        let api_resp: serde_json::Value = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

        let data = api_resp["data"].as_array().cloned().unwrap_or_default();

        let models: Vec<ModelInfo> = data
            .into_iter()
            .filter_map(|m| {
                let model_id = m["id"].as_str()?.to_string();
                Some(ModelInfo {
                    id: model_id.clone(),
                    name: format!("MiniMax {}", model_id.trim_start_matches("MiniMax-")),
                    context_window: 32_768,
                    max_tokens: 8_192,
                    default_temperature: Some(0.7),
                    reasoning: false,
                    input_types: vec![crate::llm::InputType::Text],
                })
            })
            .collect();

        Ok(models)
    }
}

#[cfg(test)]
#[path = "minimax/tests.rs"]
mod tests;
