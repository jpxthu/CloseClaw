//! MiniMax streaming chat implementation via Anthropic protocol.
//!
//! MiniMax Anthropic SSE stream format:
//! ```text
//! event: message_start
//! data: {"type":"message_start","message":{"id":"...","type":"message","role":"assistant","content":[],"model":"...","stop_reason":null,"usage":{"input_tokens":10,"output_tokens":0}}}
//!
//! event: content_block_start
//! data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}
//!
//! event: content_block_delta
//! data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}
//!
//! event: content_block_stop
//! data: {"type":"content_block_stop","index":0}
//!
//! event: message_delta
//! data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":15}}
//!
//! event: message_stop
//! data: {"type":"message_stop"}
//! ```

use tokio::sync::mpsc;

use crate::provider::{Provider, Result, SseStream};
use crate::types::RawSseChunk;

use crate::MiniMaxProvider;

/// Send a streaming chat request to MiniMax (Anthropic protocol).
///
/// Sends a POST request with the provided body and returns an
/// SSE event stream as [`SseStream`].
pub(crate) async fn send_streaming_request(
    provider: &MiniMaxProvider,
    body: serde_json::Value,
) -> Result<SseStream> {
    let response = provider
        .http_client()
        .post(provider.base_url())
        .header("x-api-key", provider.api_key())
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(MiniMaxProvider::map_status_error(status, body));
    }

    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(run_sse_stream(response, tx));
    Ok(rx)
}

/// Process an SSE byte stream from a MiniMax Anthropic-format streaming response,
/// forwarding parsed chunks to the channel.
pub(crate) async fn run_sse_stream(response: reqwest::Response, tx: mpsc::Sender<RawSseChunk>) {
    use futures::StreamExt;

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
            let (event_type, data) = parse_sse_block(&event_block);
            if !event_type.is_empty() {
                let _ = tx.send(RawSseChunk { event_type, data }).await;
            }
        }
    }

    // Process any remaining data in buffer
    if !buffer.is_empty() {
        let (event_type, data) = parse_sse_block(&buffer);
        if !event_type.is_empty() {
            let _ = tx.send(RawSseChunk { event_type, data }).await;
        }
    }
}

/// Parse a single SSE event block into (event_type, data).
fn parse_sse_block(block: &str) -> (String, String) {
    let mut event_type = String::new();
    let mut data = String::new();

    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event: ") {
            event_type = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data: ") {
            data = value.trim().to_string();
        }
    }

    (event_type, data)
}
