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
/// The handler drops the sender when it returns (body end or `[DONE]`),
/// which closes the channel, so the sequential drain below ends at channel
/// close — no collector task, no sleeping involved. Test bodies carry a
/// handful of events, well under the bounded capacity, so the awaited
/// sends never block.
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
    run_sse_stream(response, tx).await;

    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }

    mock.assert_async().await;
    chunks
}

/// Like [`collect_forwarded_chunks`], but serve the response as a chunked
/// body written from `parts`: each element becomes one HTTP chunk, which
/// hyper delivers as its own frame, so the handler observes each part in a
/// separate `stream.next()` step regardless of TCP segmentation.
async fn collect_forwarded_chunks_chunked(parts: &'static [&'static str]) -> Vec<RawSseChunk> {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/sse")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_chunked_body(move |w| {
            for part in parts {
                w.write_all(part.as_bytes())?;
            }
            Ok(())
        })
        .create_async()
        .await;

    let response = reqwest::get(format!("{}/sse", server.url()))
        .await
        .expect("loopback mock SSE response should be fetched");

    let (tx, mut rx) = mpsc::channel(64);
    run_sse_stream(response, tx).await;

    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }

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

/// `[DONE]` returns from inside the main parse path's line loop: later
/// `data: ` lines in the *same* event block share its `\n\n` terminator,
/// yet must never be forwarded once the marker line is seen.
#[tokio::test]
async fn test_glm_sse_done_skips_later_lines_in_same_block() {
    let chunks = collect_forwarded_chunks(concat!(
        "data: {\"seq\": 1}\n\n",
        "data: [DONE]\ndata: {\"seq\": 2}\n\n",
    ))
    .await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}"],
        "data lines after [DONE] inside the same event block must be skipped"
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

/// `[DONE]` in the trailing residual buffer takes the flush path's early
/// return: the marker line has no `\n\n` terminator behind it, so it is
/// only reachable through the residual-buffer branch, which must return
/// before the line following it is forwarded.
#[tokio::test]
async fn test_glm_sse_done_in_trailing_buffer_returns_early() {
    let chunks = collect_forwarded_chunks(concat!(
        "data: {\"seq\": 1}\n\n",
        "data: [DONE]\ndata: {\"seq\": 2}",
    ))
    .await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}"],
        "the residual-buffer [DONE] branch must return before forwarding later lines"
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

/// The `\n\n` separator can straddle two network chunks: the first HTTP
/// chunk ends with a lone `\n`, so the first event stays buffered until the
/// next `stream.next()` step supplies the rest. Both events must still be
/// forwarded in order — buffer carry-over across read iterations.
#[tokio::test]
async fn test_glm_sse_separator_split_across_network_chunks() {
    let chunks =
        collect_forwarded_chunks_chunked(&["data: {\"seq\": 1}\n", "\ndata: {\"seq\": 2}\n\n"])
            .await;

    assert_eq!(
        data_of(&chunks),
        vec!["{\"seq\": 1}", "{\"seq\": 2}"],
        "an event split across network chunks must be carried until complete"
    );
}

// The stream read-error branch (`Err(_) => break` in `streaming.rs`, then
// the residual-buffer flush) is intentionally left uncovered: injecting the
// error through `with_chunked_body` races against hyper's server-side
// flush. The body-error frame can be consumed before the buffered data
// frame reaches the socket, so the client sometimes receives only the
// response head and then a read error with an empty buffer (observed ~5
// failures per 100 runs). Any assertion over that path would depend on
// scheduling, which is forbidden as flaky; a deterministic injection point
// would require constructing the `reqwest::Response` body directly instead
// of going through the loopback mock.
