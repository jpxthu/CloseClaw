//! DeepSeek LLM Provider
//!
//! Uses the DeepSeek API. Supports both OpenAI (`/chat/completions`)
//! and Anthropic (`/v1/messages`) protocol formats. Model list is
//! fetched from `base_url/models`.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::HeaderMap;
use reqwest::Client;
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::provider::{Provider, ProviderError, SseStream};
use crate::types::{
    InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage,
};
use crate::{LLMError, ModelInfo, ModelLister};

/// DeepSeek API endpoint
const DEEPSEEK_API_URL: &str = "https://api.deepseek.com";

/// DeepSeek chat response body (OpenAI-compatible)
#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    message: DeepSeekMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[allow(dead_code)]
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
#[allow(dead_code)]
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

#[allow(dead_code)]
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
    supported_protocols: Vec<ProtocolId>,
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
            supported_protocols: vec![ProtocolId::new("openai"), ProtocolId::new("anthropic")],
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }

    /// Map HTTP status code to the appropriate provider error.
    fn map_status_error(status: reqwest::StatusCode, body: String) -> ProviderError {
        ProviderError::Legacy(format!("DeepSeek API error {}: {}", status, body))
    }
}

// ── Provider trait implementation ─────────────────────────────────────────────

#[async_trait]
impl Provider for DeepSeekProvider {
    fn id(&self) -> &str {
        "deepseek"
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
    ) -> crate::provider::Result<InternalResponse> {
        let is_anthropic = detect_is_anthropic(&body);
        let url = if is_anthropic {
            self.messages_url()
        } else {
            self.chat_url()
        };

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
    ) -> crate::provider::Result<SseStream> {
        let is_anthropic = detect_is_anthropic(&body);
        let url = if is_anthropic {
            self.messages_url()
        } else {
            self.chat_url()
        };

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

#[async_trait]
impl ModelLister for DeepSeekProvider {
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
            return Err(match status.as_u16() {
                401 | 403 => LLMError::AuthFailed(body),
                404 => LLMError::ModelNotFound(body),
                422 => LLMError::InvalidRequest(body),
                429 => LLMError::RateLimitExceeded,
                _ => LLMError::ApiError(format!("unexpected status {}: {}", status, body)),
            });
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
                let input_types: Vec<crate::InputType> = m
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

                // DeepSeek does not expose reasoning flag in /models.
                // Conservatively set reasoning=false.
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

// ── Protocol detection ────────────────────────────────────────────────────

/// Detect whether the request body uses Anthropic protocol format.
///
/// Anthropic messages have structured content arrays
/// (`[{"type": "text", "text": "..."}]`), while OpenAI uses plain
/// strings (`"content": "..."`).
fn detect_is_anthropic(body: &serde_json::Value) -> bool {
    body.get("messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.first())
        .and_then(|msg| msg.get("content"))
        .map(|c| c.is_array())
        .unwrap_or(false)
}

// ── OpenAI response parsing ─────────────────────────────────────────────────

async fn parse_openai_response(
    response: reqwest::Response,
) -> crate::provider::Result<InternalResponse> {
    let ds_resp: DeepSeekResponse = response.json().await.map_err(ProviderError::Reqwest)?;

    if let Some(ref err) = ds_resp.error {
        let code = err.code.as_deref().unwrap_or("");
        let msg = err.message.as_deref().unwrap_or("unknown error");
        return Err(ProviderError::Legacy(format!(
            "DeepSeek API error {}: {}",
            code, msg
        )));
    }

    let choice = ds_resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ProviderError::Legacy("no choices in DeepSeek response".to_string()))?;

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

    let usage = ds_resp.usage.unwrap_or_default();

    Ok(InternalResponse {
        content_blocks,
        usage: RawUsage {
            prompt_tokens: usage.prompt_tokens.unwrap_or(0),
            completion_tokens: usage.completion_tokens.unwrap_or(0),
            total_tokens: usage.total_tokens,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: choice.finish_reason,
    })
}

// ── Anthropic response parsing ──────────────────────────────────────────────

/// Parse an Anthropic-format response body into `InternalResponse`.
fn parse_anthropic_response(body: serde_json::Value) -> crate::provider::Result<InternalResponse> {
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

/// Parse a single Anthropic content block.
fn parse_content_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    let ty = item.get("type").and_then(|v| v.as_str())?;
    match ty {
        "text" => parse_text_block(item),
        "thinking" => parse_thinking_block(item),
        _ => None,
    }
}

fn parse_text_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    item.get("text")
        .and_then(|v| v.as_str())
        .map(|s| RawContentBlock::Text(s.to_string()))
}

fn parse_thinking_block(item: &serde_json::Value) -> Option<RawContentBlock> {
    let thinking = item.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
    let signature = item
        .get("signature")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(RawContentBlock::Thinking {
        thinking: thinking.to_string(),
        signature,
    })
}

/// Parse usage from an Anthropic response body.
fn parse_anthropic_usage(body: &serde_json::Value) -> RawUsage {
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
    let cache_write_tokens = u
        .and_then(|u| u.get("cache_creation_input_tokens"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    RawUsage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        cache_read_tokens,
        cache_write_tokens,
    }
}

// ── SSE streaming helpers ───────────────────────────────────────────────────

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

pub mod balance;

#[cfg(test)]
#[path = "deepseek/tests.rs"]
mod tests;
