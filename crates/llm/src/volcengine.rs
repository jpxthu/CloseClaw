//! VolcEngine (火山引擎) LLM Provider
//!
//! Uses the Volcano Ark (方舟) API. Chat endpoint is OpenAI-compatible at
//! `base_url/chat/completions`. Model list is fetched from `base_url/models`.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::HeaderMap;
use reqwest::Client;
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::provider::{Provider, ProviderError, SseStream};
use crate::types::{InternalRequest, ProtocolId, RawSseChunk};
use crate::{LLMError, ModelInfo, ModelLister};

/// VolcEngine API endpoint
const VOLCENGINE_API_URL: &str = "https://ARK.cn-beijing.volces.com/api/v3";

/// VolcEngine chat response body (OpenAI-compatible)
#[allow(dead_code)]
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
    #[allow(dead_code)]
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
    supported_protocols: Vec<ProtocolId>,
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
            supported_protocols: vec![ProtocolId::new("openai")],
        }
    }
}

// ── Provider trait implementation ─────────────────────────────────────────────

#[async_trait]
impl Provider for VolcEngineProvider {
    fn id(&self) -> &str {
        "volcengine"
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
        &self.http_client
    }

    fn default_headers(&self) -> &HeaderMap {
        static EMPTY: OnceLock<HeaderMap> = OnceLock::new();
        EMPTY.get_or_init(HeaderMap::new)
    }

    async fn send(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> crate::provider::Result<serde_json::Value> {
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(crate::provider::map_http_error(status, body, None));
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(ProviderError::Reqwest)
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> crate::provider::Result<SseStream> {
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_after = crate::provider::parse_retry_after(response.headers());
            let body = response.text().await.unwrap_or_default();
            return Err(crate::provider::map_http_error(status, body, retry_after));
        }

        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(_) => break,
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE events (separated by \n\n)
                while let Some(pos) = buffer.find("\n\n") {
                    let event_block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    for line in event_block.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim().to_string();
                            if data == "[DONE]" {
                                return;
                            }
                            let _ = tx
                                .send(RawSseChunk {
                                    event_type: "message".into(),
                                    data,
                                })
                                .await;
                        }
                    }
                }
            }

            // Process any remaining data in buffer
            if !buffer.is_empty() {
                for line in buffer.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        let data = data.trim().to_string();
                        if data == "[DONE]" {
                            return;
                        }
                        let _ = tx
                            .send(RawSseChunk {
                                event_type: "message".into(),
                                data,
                            })
                            .await;
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[async_trait]
impl ModelLister for VolcEngineProvider {
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
            return Err(LLMError::from(&crate::provider::map_http_error(
                status, body, None,
            )));
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
                let input_types: Vec<crate::InputType> = props
                    .input_modalities
                    .iter()
                    .map(|m| match m.to_lowercase().as_str() {
                        "image" => crate::InputType::Image,
                        _ => crate::InputType::Text,
                    })
                    .collect();
                let input_types = if input_types.is_empty() {
                    vec![crate::InputType::Text]
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
                    reasoning_levels: crate::ReasoningLevels::None,
                    input_types,
                }
            })
            .collect();

        Ok(models)
    }
}

#[cfg(test)]
#[path = "volcengine/tests.rs"]
mod tests;
