//! MiniMax LLM Provider — pure HTTP transport for the
//! MiniMax Chat Completions API.

use crate::provider::{Provider, ProviderError, Result, SseStream};
use crate::types::{InternalRequest, ProtocolId};
use crate::{LLMError, ModelInfo, ModelLister};
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;

use std::sync::OnceLock;

#[path = "minimax_stream.rs"]
pub(crate) mod minimax_stream;
pub(crate) mod plugin;
pub use plugin::{MiniMaxM2Plugin, MiniMaxM3Plugin};

// ---------------------------------------------------------------------------//
// Constants                                                                  //
// ---------------------------------------------------------------------------//

const MINIMAX_API_URL: &str = "https://api.minimax.chat/v1/messages";

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

    /// Map MiniMax internal base_resp status_code to ProviderError.
    pub(crate) fn map_base_resp_error(status_code: i32, status_msg: &str) -> ProviderError {
        ProviderError::Legacy(format!(
            "MiniMax business error {}: {}",
            status_code, status_msg
        ))
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
    ) -> Result<serde_json::Value> {
        let response = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::provider::map_http_error(status, body, None));
        }

        let value: serde_json::Value = response.json().await.map_err(ProviderError::Reqwest)?;

        // Check MiniMax business errors (base_resp.status_code != 0)
        if let Some(base_resp) = value.get("base_resp") {
            if let Some(code) = base_resp.get("status_code") {
                if code.as_i64().unwrap_or(0) != 0 {
                    let msg = base_resp
                        .get("status_msg")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown");
                    return Err(Self::map_base_resp_error(
                        code.as_i64().unwrap_or(0) as i32,
                        msg,
                    ));
                }
            }
        }

        Ok(value)
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
                ));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(LLMError::from(&crate::provider::map_http_error(
                status, body, None,
            )));
        }

        let api_resp: serde_json::Value = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;

        let data = api_resp["data"].as_array().cloned().unwrap_or_default();

        let kb = crate::ProviderModelKnowledge::new();
        let models: Vec<ModelInfo> = data
            .into_iter()
            .filter_map(|m| {
                let model_id = m["id"].as_str()?.to_string();
                let params = kb.find("minimax", &model_id);
                let (
                    context_window,
                    max_tokens,
                    default_temperature,
                    reasoning,
                    reasoning_levels,
                    input_types,
                ) = match params {
                    Some(p) => (
                        p.context_window,
                        p.max_tokens,
                        Some(p.default_temperature),
                        p.reasoning,
                        p.reasoning_levels,
                        p.input_types,
                    ),
                    None => (
                        32_768,
                        8_192,
                        Some(0.7),
                        false,
                        crate::ReasoningLevels::None,
                        vec![crate::InputType::Text],
                    ),
                };
                Some(ModelInfo {
                    id: model_id.clone(),
                    name: format!("MiniMax {}", model_id.trim_start_matches("MiniMax-")),
                    context_window,
                    max_tokens,
                    default_temperature,
                    reasoning,
                    reasoning_levels,
                    input_types,
                })
            })
            .collect();

        Ok(models)
    }
}

#[cfg(test)]
#[path = "minimax/tests.rs"]
mod tests;
