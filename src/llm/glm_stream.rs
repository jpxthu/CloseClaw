//! GLM streaming chat implementation.
//!
//! GLM SSE stream format:
//! ```text
//! data: {"id":"...","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"..."}}]}
//! data: {"id":"...","choices":[{"finish_reason":"length","index":0,"delta":{"role":"assistant","content":""}}],"usage":{"prompt_tokens":9,"completion_tokens":30,"total_tokens":39,"prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":30}}}
//! data: [DONE]
//! ```
//! - Delta chunks contain `delta.reasoning_content` (or `delta.content`)
//! - The final chunk has `finish_reason` non-null, `delta` still present but empty,
//!   plus `usage` at chunk level
//! - `[DONE]` is the termination marker
//! - GLM has no `base_resp`; errors come via HTTP status or JSON `error` field

use serde::Deserialize;

use crate::llm::{ChatRequest, ChatStreamChunk, GlmProvider, LLMError, StreamingResponse, Usage};

use serde::Serialize;

/// GLM streaming request body with `stream: true`.
#[derive(Debug, Serialize)]
pub(crate) struct GlmStreamRequest<'a> {
    model: &'a str,
    messages: &'a [crate::llm::Message],
    temperature: f32,
    max_tokens: Option<u32>,
    stream: bool,
}

/// A single SSE chunk from GLM streaming API.
#[derive(Debug, Deserialize)]
pub(crate) struct GlmStreamChunk {
    #[serde(default)]
    choices: Option<Vec<GlmStreamChoice>>,
    #[serde(default)]
    usage: Option<GlmStreamUsage>,
    #[serde(default)]
    model: String,
    /// GLM top-level error (rare in streaming, but possible on early errors)
    #[serde(default)]
    error: Option<GlmStreamError>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmStreamChoice {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    delta: Option<GlmStreamDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmStreamDelta {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmStreamUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
    #[serde(default)]
    completion_tokens_details: Option<GlmStreamCompletionTokensDetails>,
    #[serde(default)]
    prompt_tokens_details: Option<GlmStreamPromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmStreamCompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmStreamPromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmStreamError {
    code: String,
    message: String,
}

/// Send a streaming chat request to GLM.
///
/// Builds a POST request with `stream: true`, parses SSE responses,
/// and yields delta chunks via an mpsc channel.
pub(crate) async fn send_streaming_request(
    provider: &GlmProvider,
    request: ChatRequest,
) -> Result<StreamingResponse, LLMError> {
    let (tx, rx) = tokio::sync::mpsc::channel(32);

    let stream_request = GlmStreamRequest {
        model: &request.model,
        messages: &request.messages,
        temperature: request.temperature,
        max_tokens: request.max_tokens,
        stream: true,
    };

    let response = provider
        .http_client
        .post(&provider.base_url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&stream_request)
        .send()
        .await
        .map_err(|e| LLMError::NetworkError(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(GlmProvider::map_status_error(status, body));
    }

    // Spawn a background task to read the SSE stream
    tokio::spawn(async move {
        if let Err(e) = read_sse_stream(response, tx).await {
            tracing::warn!("GLM streaming error: {}", e);
        }
    });

    Ok(rx)
}

/// Read the SSE stream from a successful HTTP response.
async fn read_sse_stream(
    mut response: reqwest::Response,
    tx: tokio::sync::mpsc::Sender<ChatStreamChunk>,
) -> Result<(), LLMError> {
    let mut buffer: Vec<u8> = Vec::new();

    loop {
        match response.chunk().await {
            Ok(Some(bytes)) => {
                buffer.extend_from_slice(&bytes);

                // Process all complete lines in the buffer
                let consumed = process_buffer(&buffer, &tx)?;

                // Remove consumed bytes, keeping any partial line
                buffer.drain(..consumed);
            }
            Ok(None) => {
                // Stream ended naturally — close channel
                drop(tx);
                return Ok(());
            }
            Err(e) => {
                let err = LLMError::NetworkError(e.to_string());
                let _ = tx.send(ChatStreamChunk::Error(err.clone())).await;
                return Err(err);
            }
        }
    }
}

/// Process all complete SSE lines in the buffer.
/// Returns the number of bytes consumed.
pub(crate) fn process_buffer(
    buffer: &[u8],
    tx: &tokio::sync::mpsc::Sender<ChatStreamChunk>,
) -> Result<usize, LLMError> {
    let s = std::str::from_utf8(buffer)
        .map_err(|e| LLMError::ApiError(format!("invalid UTF-8 in stream: {}", e)))?;

    let mut consumed = 0;

    for line in s.lines() {
        let line_len = line.len() + 1; // +1 for the \n separator
        consumed += line_len;

        let Some(data) = parse_sse_line(line) else {
            // Lines not starting with "data:" are ignored (SSE comments, blank lines, etc.)
            continue;
        };

        if data == "[DONE]" {
            // End of stream — close channel
            return Ok(consumed);
        }

        let chunk = parse_stream_chunk(data)?;
        if !process_chunk(chunk, tx)? {
            // Final chunk processed — will close on the next read
            continue;
        }
    }

    Ok(consumed.min(s.len()))
}

/// Parse an SSE line: `data: {...}` or `data: [DONE]`.
/// Returns None for non-SSE lines or empty lines.
pub(crate) fn parse_sse_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.starts_with("data:") {
        return None;
    }
    let data = trimmed[5..].trim(); // strip "data:"
    if data == "[DONE]" {
        return Some("[DONE]");
    }
    if !data.starts_with('{') {
        return None;
    }
    Some(data)
}

/// Parse a GLM stream chunk JSON blob.
pub(crate) fn parse_stream_chunk(json_data: &str) -> Result<GlmStreamChunk, LLMError> {
    serde_json::from_str::<GlmStreamChunk>(json_data)
        .map_err(|e| LLMError::ApiError(format!("failed to parse stream chunk: {}", e)))
}

/// Process a single stream chunk, sending results through the channel.
/// Returns Ok(true) if the stream should continue, Ok(false) if it's the final chunk.
pub(crate) fn process_chunk(
    chunk: GlmStreamChunk,
    tx: &tokio::sync::mpsc::Sender<ChatStreamChunk>,
) -> Result<bool, LLMError> {
    // Check GLM top-level error (may appear on early-stream errors)
    if let Some(ref err) = chunk.error {
        return Err(GlmProvider::map_glm_error(&err.code, &err.message));
    }

    let choices = match chunk.choices {
        Some(c) => c,
        None => return Ok(true),
    };

    for choice in &choices {
        // Final chunk: has `finish_reason` non-null and `usage` at chunk level
        if choice.finish_reason.is_some() {
            // The delta in final chunk is empty; emit no text for this choice
            // (reasoning content already came in previous delta chunks)

            let usage = chunk
                .usage
                .as_ref()
                .map(|u| Usage {
                    prompt_tokens: u.prompt_tokens,
                    completion_tokens: u.completion_tokens,
                    total_tokens: u.total_tokens,
                })
                .unwrap_or_default();

            let _ = tx.try_send(ChatStreamChunk::Done {
                model: chunk.model,
                usage,
            });

            return Ok(false);
        }

        // Delta chunk: extract text content
        // Per plan: content is preferred; reasoning_content used only when content is empty
        if let Some(ref delta) = choice.delta {
            let text = if delta.content.is_some() && !delta.content.as_ref().unwrap().is_empty() {
                delta.content.as_ref().unwrap().clone()
            } else {
                delta
                    .reasoning_content
                    .as_ref()
                    .unwrap_or(&String::new())
                    .clone()
            };

            if !text.is_empty() {
                let _ = tx.try_send(ChatStreamChunk::Text(text));
            }
        }
    }

    Ok(true)
}

#[cfg(test)]
mod tests;
