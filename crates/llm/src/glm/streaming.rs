//! GLM SSE streaming handler.

use tokio::sync::mpsc;

use crate::types::RawSseChunk;

/// Process an SSE byte stream from a GLM streaming response,
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
