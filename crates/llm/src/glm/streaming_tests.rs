//! Unit tests for the GLM SSE byte-stream handler (`run_sse_stream`).
//!
//! Each test serves a canned SSE body from a loopback mockito server, runs
//! `run_sse_stream` to completion over the resulting `reqwest::Response`, and
//! asserts the forwarded `RawSseChunk`s. No external network access
//! (STANDARDS §5).

use tokio::sync::mpsc;

use super::run_sse_stream;
use crate::types::RawSseChunk;

/// Serve `body` as an SSE response on a loopback mock server, run
/// `run_sse_stream` to completion, and return every chunk it forwarded.
///
/// A spawned collector drains the channel while the handler runs, so the
/// bounded capacity never limits the event count. The handler drops the
/// sender when it returns (body end or `[DONE]`), which closes the channel
/// and ends the collector — completion is signaled by channel close, no
/// sleeping involved.
async fn collect_forwarded_chunks(body: &str) -> Vec<RawSseChunk> {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/sse")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let response = reqwest::get(format!("{}/sse", server.url()))
        .await
        .expect("loopback mock SSE response should be fetched");

    let (tx, mut rx) = mpsc::channel(64);
    let collector = tokio::spawn(async move {
        let mut chunks = Vec::new();
        while let Some(chunk) = rx.recv().await {
            chunks.push(chunk);
        }
        chunks
    });

    run_sse_stream(response, tx).await;
    let chunks = collector.await.expect("collector task should finish");

    mock.assert_async().await;
    chunks
}

fn data_of(chunks: &[RawSseChunk]) -> Vec<&str> {
    chunks.iter().map(|chunk| chunk.data.as_str()).collect()
}

/// Normal `data: ` event line parsing: a line whose prefix contains the space
/// after the colon is forwarded as one `event_type="message"` chunk, with
/// surrounding whitespace trimmed from the payload.
#[tokio::test]
async fn test_glm_sse_parses_data_line_with_space_prefix() {
    let chunks = collect_forwarded_chunks("data:   {\"seq\": 1}   \n\n").await;

    assert_eq!(
        chunks.len(),
        1,
        "a single data line should forward exactly one chunk"
    );
    assert_eq!(
        chunks[0].event_type, "message",
        "forwarded chunks carry the fixed \"message\" event type"
    );
    assert_eq!(
        chunks[0].data, "{\"seq\": 1}",
        "payload surrounding whitespace should be trimmed"
    );
}

/// `[DONE]` terminates the stream: only events before the marker are
/// forwarded, the event after it is dropped, and the handler returns
/// (the awaited run completes).
#[tokio::test]
async fn test_glm_sse_done_terminates_stream() {
    let chunks = collect_forwarded_chunks(concat!(
        "data: {\"seq\": 1}\n\n",
        "data: [DONE]\n\n",
        "data: {\"seq\": 2}\n\n",
    ))
    .await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}"],
        "events after [DONE] must not be forwarded"
    );
}

/// Multiple complete events in the buffer are split on the `\n\n` separator
/// and forwarded as separate chunks, in arrival order.
#[tokio::test]
async fn test_glm_sse_multiple_events_split_on_blank_lines() {
    let chunks = collect_forwarded_chunks(concat!(
        "data: {\"seq\": 1}\n\n",
        "data: {\"seq\": 2}\n\n",
        "data: {\"seq\": 3}\n\n",
    ))
    .await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}", "{\"seq\": 2}", "{\"seq\": 3}"],
        "each blank-line-delimited event should be forwarded in order"
    );
}

/// One event block with multiple lines forwards only its `data: ` line;
/// `event:` / `id:` lines inside the same block produce no chunks.
#[tokio::test]
async fn test_glm_sse_single_block_forwards_only_data_line() {
    let chunks = collect_forwarded_chunks("event: message\nid: 7\ndata: {\"seq\": 1}\n\n").await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}"],
        "non-data lines inside the block must not produce chunks"
    );
}

/// Trailing buffer without a `\n\n` terminator is flushed after the read
/// loop: the final event has no blank-line terminator behind it, so it can
/// only be forwarded by the residual-buffer path.
#[tokio::test]
async fn test_glm_sse_flushes_trailing_buffer_without_terminator() {
    let chunks = collect_forwarded_chunks("data: {\"seq\": 1}\n\ndata: {\"seq\": 2}").await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}", "{\"seq\": 2}"],
        "the unterminated trailing event should still be forwarded"
    );
}

/// Non-data lines and empty blocks are ignored: comment lines (`:`),
/// `event:` fields, `data:` without the space prefix, and stray blank
/// separators never produce chunks — only proper `data: ` lines do.
#[tokio::test]
async fn test_glm_sse_ignores_non_data_and_empty_lines() {
    let body = concat!(
        "event: message\n\n",
        ": comment\n",
        "data:{\"seq\": 0}\n\n",
        "data: {\"seq\": 1}\n\n",
        "\n\n",
        "data: {\"seq\": 2}\n\n",
    );
    let chunks = collect_forwarded_chunks(body).await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}", "{\"seq\": 2}"],
        "comment/event/no-space lines and empty blocks must be skipped"
    );
}
