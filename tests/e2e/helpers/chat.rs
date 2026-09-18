//! Shared chat-RPC client helpers for e2e tests.
//!
//! Centralizes the length-prefixed JSON frame protocol used against
//! `chat.sock` (and, via [`read_frame`], `admin.sock`) so test cases
//! don't re-implement the wire format.
//!
//! Frame format (mirrors `crates/cli/src/chat/rpc/protocol.rs`):
//! `[4-byte big-endian u32 length][JSON frame bytes]`.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream as TokioUnixStream;
use tokio::time::timeout;

/// Upper bound for one full chat turn (request → Done/Error/EOF).
///
/// Deliberately well under the daemon's `TURN_COMPLETION_TIMEOUT_SECS`
/// (120s): a turn that only completes by riding out the completion
/// timeout trips this bound first — proving completion is driven by the
/// turn-completion consumer, not the timeout (see
/// `gateway_restart_turn_tests`).
pub const CHAT_TURN_TIMEOUT: Duration = Duration::from_secs(60);

/// Send one `ChatMessage` and collect frames until `Done`/`Error`/EOF,
/// returning the frames (Step 1.23 — the Result-typed primitive).
///
/// Connect / I/O / per-turn-timeout failures are returned as `Err` so
/// polling callers (e.g. `wait_post_restart_turn`) can retry them until
/// their deadline instead of panicking mid-window. Direct call sites
/// use [`chat_roundtrip`], the panic-on-failure wrapper.
pub async fn try_chat_roundtrip(
    socket_path: &Path,
    agent_id: &str,
    content: &str,
) -> std::io::Result<Vec<serde_json::Value>> {
    let stream = TokioUnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();

    let request = serde_json::json!({
        "type": "chat_message",
        "agent_id": agent_id,
        "content": content,
    });
    let body = serde_json::to_vec(&request).expect("serialize chat request");
    let header = (body.len() as u32).to_be_bytes();
    writer.write_all(&header).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;

    let mut reader = BufReader::new(reader);
    let mut frames = Vec::new();
    loop {
        let frame = match timeout(CHAT_TURN_TIMEOUT, read_frame(&mut reader)).await {
            Ok(Ok(Some(f))) => f,
            Ok(Ok(None)) => break, // EOF — server closed the connection
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("chat turn timed out after {CHAT_TURN_TIMEOUT:?}"),
                ));
            }
        };
        let is_terminal = frame.get("type").and_then(|t| t.as_str()) == Some("done")
            || frame.get("type").and_then(|t| t.as_str()) == Some("error");
        frames.push(frame);
        if is_terminal {
            break;
        }
    }
    Ok(frames)
}

/// Panic-on-failure wrapper around [`try_chat_roundtrip`] for direct
/// call sites that do not retry (pre-restart sanity turn, agent-profile
/// cases): any connect / I/O / timeout failure fails the test.
pub async fn chat_roundtrip(
    socket_path: &Path,
    agent_id: &str,
    content: &str,
) -> Vec<serde_json::Value> {
    try_chat_roundtrip(socket_path, agent_id, content)
        .await
        .unwrap_or_else(|e| panic!("chat roundtrip via {} failed: {e}", socket_path.display()))
}

/// Concatenated `content` fragments of all frames in `frames` — the
/// text the chat client rendered for this turn.
pub fn collect_content_text(frames: &[serde_json::Value]) -> String {
    frames
        .iter()
        .filter_map(|f| f.get("content").and_then(|c| c.as_str()))
        .collect()
}

/// Assert exactly one terminal frame (`done` or `error`) in a set of
/// chat response frames.
pub fn assert_single_terminal(frames: &[serde_json::Value]) {
    let terminal: Vec<&serde_json::Value> = frames
        .iter()
        .filter(|f| {
            matches!(
                f.get("type").and_then(|t| t.as_str()),
                Some("done") | Some("error")
            )
        })
        .collect();
    assert_eq!(
        terminal.len(),
        1,
        "expected exactly one terminal (Done/Error) frame, got {terminal:?} among {frames:?}"
    );
}

/// Read one length-prefixed JSON frame. `Ok(None)` on clean EOF.
pub async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<serde_json::Value>> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(header) as usize;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    let value = serde_json::from_slice(&body).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid frame JSON: {e}"),
        )
    })?;
    Ok(Some(value))
}
