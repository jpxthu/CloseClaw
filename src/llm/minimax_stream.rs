//! MiniMax streaming chat implementation.
//!
//! MiniMax SSE stream format:
//! ```text
//! data: {"id":"...","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"..."}}]}
//! data: {"id":"...","choices":[{"finish_reason":"length","index":0,"delta":{"role":"assistant","reasoning_content":"..."}}]}
//! data: {"id":"...","choices":[{"finish_reason":"length","index":0,"message":{...}}]}
//! data: [DONE]
//! ```
//! - Delta chunks contain `delta.reasoning_content` (or `delta.content`)
//! - The final chunk contains `message` (not `delta`) with full fields + `usage` + `base_resp`
//! - `[DONE]` is the termination marker

use tokio::sync::mpsc;

use crate::llm::provider::{Provider, Result, SseStream};
use crate::llm::types::RawSseChunk;

use crate::llm::MiniMaxProvider;

/// Send a streaming chat request to MiniMax.
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
        .header("Authorization", format!("Bearer {}", provider.api_key()))
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

/// Process an SSE byte stream from a MiniMax streaming response,
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
}
