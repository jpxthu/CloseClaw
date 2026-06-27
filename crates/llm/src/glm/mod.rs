//! GLM LLM Provider — pure HTTP transport for the
//! GLM Chat Completions API.

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use std::sync::OnceLock;
use tokio::sync::mpsc;

use crate::provider::{Provider, ProviderError, Result, SseStream};
use crate::types::{InternalRequest, InternalResponse, ProtocolId};

mod models;
mod quota;
mod streaming;
mod types;

#[allow(unused_imports)]
pub(crate) use crate::types::{RawContentBlock, RawSseChunk, RawUsage};
#[allow(unused_imports)]
pub(crate) use crate::{ModelInfo, ModelLister};
use streaming::run_sse_stream;
pub(crate) use types::*;

/// GLM API endpoint (chat completions)
const GLM_CHAT_URL: &str = "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions";

// ── Provider struct ───────────────────────────────────────────────────────────

pub struct GlmProvider {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) client: Client,
    supported_protocols: Vec<ProtocolId>,
}

impl GlmProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, GLM_CHAT_URL.to_string())
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(std::env::var("GLM_API_KEY").ok()?))
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            base_url,
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("openai")],
        }
    }
}

// ── Provider trait implementation ─────────────────────────────────────────────

#[async_trait]
impl Provider for GlmProvider {
    fn id(&self) -> &str {
        "glm"
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

        let api_resp: GlmResponse = response.json().await.map_err(ProviderError::Reqwest)?;

        Self::parse_chat_response(api_resp)
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream> {
        let response = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(run_sse_stream(response, tx));
        Ok(rx)
    }
}

#[cfg(test)]
mod tests;
