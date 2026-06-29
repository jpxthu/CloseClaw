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

// ── Protocol detection ────────────────────────────────────────────────────

/// Detect whether the request body uses Anthropic protocol format.
///
/// Anthropic messages have structured content arrays
/// (`[{"type": "text", "text": "..."}]`), while OpenAI uses plain
/// strings (`"content": "..."`).
///
/// As a secondary heuristic, the presence of a top-level `system` field
/// (used only by the Anthropic protocol) is treated as a strong signal.
fn detect_is_anthropic(body: &serde_json::Value) -> bool {
    if body
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.first())
        .and_then(|msg| msg.get("content"))
        .map(|c| c.is_array())
        .unwrap_or(false)
    {
        return true;
    }

    if body.get("system").is_some() {
        return true;
    }

    false
}

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
            supported_protocols: vec![ProtocolId::new("openai"), ProtocolId::new("anthropic")],
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
            supported_protocols: vec![ProtocolId::new("openai"), ProtocolId::new("anthropic")],
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn messages_url(&self) -> String {
        format!("{}/messages", self.base_url)
    }

    /// Map HTTP status code to the appropriate provider error.
    fn map_status_error(status: reqwest::StatusCode, body: String) -> ProviderError {
        ProviderError::Legacy(format!("MiMo API error {}: {}", status, body))
    }
}

// ── SSE buffer processing helpers ─────────────────────────────────────────

/// Process complete SSE events from the buffer.
#[allow(clippy::ptr_arg)]
async fn process_sse_buffer(
    buffer: &mut String,
    current_event_type: &mut String,
    tx: &mpsc::Sender<RawSseChunk>,
) {
    while let Some(pos) = buffer.find("\n\n") {
        let event_block = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();
        for line in event_block.lines() {
            if let Some(evt) = line.strip_prefix("event: ") {
                *current_event_type = evt.trim().to_string();
            } else if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim().to_string();
                if data == "[DONE]" {
                    return;
                }
                let _ = tx
                    .send(RawSseChunk {
                        event_type: current_event_type.clone(),
                        data,
                    })
                    .await;
            }
        }
    }
}

/// Process remaining data in the buffer after stream ends.
async fn process_sse_buffer_remainder(
    buffer: &str,
    current_event_type: &str,
    tx: &mpsc::Sender<RawSseChunk>,
) {
    if !buffer.is_empty() {
        for line in buffer.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim().to_string();
                if data == "[DONE]" {
                    return;
                }
                let _ = tx
                    .send(RawSseChunk {
                        event_type: current_event_type.to_string(),
                        data,
                    })
                    .await;
            }
        }
    }
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
    /// Thinking/reasoning content (OpenAI protocol).
    /// Always present in MiMo responses when thinking is enabled.
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MimoUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

// ── OpenAI response parsing ─────────────────────────────────────────────────

/// Parse an OpenAI-format response into `InternalResponse`.
///
/// Extracts `reasoning_content` from the message into a `Thinking` block
/// (with `signature: None`, matching OpenAI convention).
pub(crate) async fn parse_openai_response(
    response: reqwest::Response,
) -> crate::provider::Result<InternalResponse> {
    let mimo_resp: MimoResponse = response.json().await.map_err(ProviderError::Reqwest)?;

    let choice = mimo_resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ProviderError::Legacy("no choices in MiMo response".to_string()))?;

    let mut content_blocks = Vec::new();

    if let Some(reasoning) = choice.message.reasoning_content {
        if !reasoning.is_empty() {
            content_blocks.push(RawContentBlock::Thinking {
                thinking: reasoning,
                signature: None,
            });
        }
    }

    if !choice.message.content.is_empty() {
        content_blocks.push(RawContentBlock::Text(choice.message.content));
    }

    if content_blocks.is_empty() {
        content_blocks.push(RawContentBlock::Text(String::new()));
    }

    Ok(InternalResponse {
        content_blocks,
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

// ── Anthropic response parsing ──────────────────────────────────────────────

/// Parse an Anthropic-format response body into `InternalResponse`.
///
/// Handles `content` array with `text` and `thinking` blocks.
/// Thinking blocks always use `signature: Some(String::new())` per MiMo docs.
pub(crate) fn parse_anthropic_response(
    body: serde_json::Value,
) -> crate::provider::Result<InternalResponse> {
    let content_blocks: Vec<RawContentBlock> = body
        .get("content")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_content_block).collect())
        .unwrap_or_default();

    let usage = parse_anthropic_usage(&body);
    let finish_reason = body
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(InternalResponse {
        content_blocks,
        usage,
        finish_reason,
    })
}

/// Parse a single Anthropic content block from a JSON value.
pub(crate) fn parse_content_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    let ty = item.get("type").and_then(|v| v.as_str())?;
    match ty {
        "text" => {
            let text = item.get("text").and_then(|v| v.as_str())?;
            Some(RawContentBlock::Text(text.to_string()))
        }
        "thinking" => {
            let thinking = item.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
            // MiMo docs: signature is always an empty string for Anthropic protocol
            Some(RawContentBlock::Thinking {
                thinking: thinking.to_string(),
                signature: Some(String::new()),
            })
        }
        _ => None,
    }
}

/// Parse usage from an Anthropic response body.
///
/// Maps `input_tokens`, `output_tokens`, and `cache_read_input_tokens`.
pub(crate) fn parse_anthropic_usage(body: &serde_json::Value) -> RawUsage {
    let u = body.get("usage");
    let input_tokens = u
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let output_tokens = u
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let total_tokens = u
        .and_then(|u| u.get("total_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let cache_read_tokens = u
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    RawUsage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        cache_read_tokens,
        cache_write_tokens: None,
    }
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
        let is_anthropic = detect_is_anthropic(&body);
        let url = if is_anthropic {
            self.messages_url()
        } else {
            self.chat_url()
        };

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

        if is_anthropic {
            let resp_body: serde_json::Value =
                response.json().await.map_err(ProviderError::Reqwest)?;
            parse_anthropic_response(resp_body)
        } else {
            parse_openai_response(response).await
        }
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream> {
        let is_anthropic = detect_is_anthropic(&body);
        let url = if is_anthropic {
            self.messages_url()
        } else {
            self.chat_url()
        };

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
            let mut current_event_type = String::from("message");

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(_) => break,
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                process_sse_buffer(&mut buffer, &mut current_event_type, &tx).await;
            }
            process_sse_buffer_remainder(&buffer, &current_event_type, &tx).await;
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod mimo_tests;
