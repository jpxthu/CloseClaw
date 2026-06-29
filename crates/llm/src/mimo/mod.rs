//! MiMo LLM Provider — pure HTTP transport for the MiMo Chat Completions API.
//!
//! MiMo uses the OpenAI-compatible protocol with a dedicated base URL.
//! The `reasoning_content` field is always present in responses (thinking is
//! always enabled by default).

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::HeaderMap;
use reqwest::Client;
use serde::Deserialize;
use std::sync::OnceLock;
use tokio::sync::mpsc;

use crate::provider::{Provider, ProviderError, Result, SseStream};
use crate::types::{
    InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage,
};

const MIMO_BASE_URL: &str = "https://api.xiaomimimo.com/v1";

// ── Provider struct ───────────────────────────────────────────────────────────

pub struct MimoProvider {
    api_key: String,
    base_url: String,
    client: Client,
    supported_protocols: Vec<ProtocolId>,
}

impl MimoProvider {
    /// Create a new `MimoProvider` with the default base URL.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: MIMO_BASE_URL.to_string(),
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("openai")],
        }
    }

    /// Create a `MimoProvider` from the `MIMO_API_KEY` environment variable.
    ///
    /// Returns `None` if the variable is not set or empty.
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("MIMO_API_KEY").ok()?;
        if key.is_empty() {
            return None;
        }
        Some(Self::new(key))
    }

    /// Create a `MimoProvider` with a custom base URL.
    pub fn with_base_url(api_key: String, base_url: &str) -> Self {
        Self {
            api_key,
            base_url: base_url.to_string(),
            client: Client::new(),
            supported_protocols: vec![ProtocolId::new("openai")],
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// Map HTTP status code to the appropriate provider error.
    fn map_status_error(status: reqwest::StatusCode, body: String) -> ProviderError {
        ProviderError::Legacy(format!("MiMo API error {}: {}", status, body))
    }
}

// ── SSE buffer processing helper ──────────────────────────────────────────────

/// Process complete SSE events in `buffer`, sending them through `tx`.
/// Returns `true` if a `[DONE]` sentinel was encountered (caller should stop).
async fn process_sse_buffer(buffer: &mut String, tx: &mpsc::Sender<RawSseChunk>) -> bool {
    while let Some(pos) = buffer.find("\n\n") {
        let event_block = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();

        for line in event_block.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim().to_string();
                if data == "[DONE]" {
                    return true;
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
    false
}

// ── Raw MiMo API response types (OpenAI-compatible) ──────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MimoResponse {
    id: String,
    model: String,
    choices: Vec<MimoChoice>,
    usage: MimoUsage,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MimoChoice {
    finish_reason: Option<String>,
    message: MimoMessage,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MimoMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct MimoUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

// ── Provider trait implementation ─────────────────────────────────────────────

#[async_trait]
impl Provider for MimoProvider {
    fn id(&self) -> &str {
        "mimo"
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
        let url = self.chat_url();

        let response = self
            .client
            .post(&url)
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

        let mimo_resp: MimoResponse = response.json().await.map_err(ProviderError::Reqwest)?;

        let choice = mimo_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::Legacy("no choices in MiMo response".to_string()))?;

        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text(choice.message.content)],
            usage: RawUsage {
                prompt_tokens: mimo_resp.usage.prompt_tokens,
                completion_tokens: mimo_resp.usage.completion_tokens,
                total_tokens: Some(mimo_resp.usage.total_tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: choice.finish_reason,
        })
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream> {
        let url = self.chat_url();

        let response = self
            .client
            .post(&url)
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
                if process_sse_buffer(&mut buffer, &tx).await {
                    return;
                }
            }

            if process_sse_buffer(&mut buffer, &tx).await {
                return;
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod mimo_tests;
