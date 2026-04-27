//! Integration tests for ChatSession
//!
//! These tests verify session behaviour through the public API.
//! Shared setup helpers live in `src/chat/session.rs` (pub(crate)).

use closeclaw::chat::protocol::ServerMessage;
use closeclaw::chat::session::ChatSession;
use closeclaw::llm::{LLMRegistry, Message, StubProvider};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Shared setup
// ---------------------------------------------------------------------------

/// Set up a `ChatSession` backed by a real TCP pair with `StubProvider`
/// registered in the `LLMRegistry`.
///
/// Returns `(session, client_stream)` where `client_stream` is the write/read
/// half connected to the session.
///
/// Note: This creates a temporary shutdown channel that will be dropped,
/// causing the session to receive a shutdown signal. For tests that need
/// to keep the session alive, use `setup_session_with_shutdown_tx` instead.
async fn setup_session() -> (ChatSession, tokio::net::TcpStream) {
    std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("stub".to_string(), Arc::new(StubProvider::new()))
        .await;
    let session = ChatSession::new(
        "test-session".to_string(),
        "test-agent".to_string(),
        accepted,
        shutdown_rx.resubscribe(),
        registry,
    );
    std::env::remove_var("LLM_FALLBACK_CHAIN");
    (session, client)
}

/// Set up a `ChatSession` with an explicit shutdown channel.
/// Returns `(session, client_stream, shutdown_tx)` where `shutdown_tx` must
/// be kept alive to prevent premature shutdown.
async fn setup_session_with_shutdown_tx(
) -> (ChatSession, tokio::net::TcpStream, broadcast::Sender<()>) {
    std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(2);
    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("stub".to_string(), Arc::new(StubProvider::new()))
        .await;
    let session = ChatSession::new(
        "test-session".to_string(),
        "test-agent".to_string(),
        accepted,
        shutdown_rx,
        registry,
    );
    std::env::remove_var("LLM_FALLBACK_CHAIN");
    (session, client, shutdown_tx)
}

/// Drain a single JSON line from `reader` and parse it as `ServerMessage`.
async fn read_server_message(
    reader: &mut tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
) -> ServerMessage {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let trimmed = line.trim();
    serde_json::from_str(trimmed)
        .unwrap_or_else(|e| panic!("failed to parse server message from `{trimmed}`: {e}"))
}

/// Send a raw JSON client message (caller is responsible for valid JSON).
async fn send_client_json(writer: &mut tokio::net::tcp::OwnedWriteHalf, json: &str) {
    writer.write_all(json.as_bytes()).await.unwrap();
    writer.write_all(b"\n").await.unwrap();
    writer.flush().await.unwrap();
}

// ---------------------------------------------------------------------------
// truncate_history — now calls the actual method on ChatSession
// ---------------------------------------------------------------------------

/// Verify that `truncate_history` removes the oldest entries when the history
/// exceeds `max_history`.
#[tokio::test]
async fn test_truncate_history_removes_oldest() {
    let (mut session, _client) = setup_session().await;
    session.max_history = 5;
    session.chat_history = (0..10_u32)
        .map(|i| Message {
            role: "user".to_string(),
            content: format!("message {}", i),
        })
        .collect();

    session.truncate_history();

    assert_eq!(session.chat_history.len(), 5);
    assert_eq!(session.chat_history[0].content, "message 5");
    assert_eq!(session.chat_history[4].content, "message 9");
}

/// Verify that `truncate_history` is a no-op when the history is under the limit.
#[tokio::test]
async fn test_truncate_history_under_limit() {
    let (mut session, _client) = setup_session().await;
    session.max_history = 5;
    session.chat_history = (0..3_u32)
        .map(|i| Message {
            role: "user".to_string(),
            content: format!("message {}", i),
        })
        .collect();

    session.truncate_history();

    assert_eq!(session.chat_history.len(), 3);
}

/// Verify that `truncate_history` is a no-op when the history is exactly at
/// the limit.
#[tokio::test]
async fn test_truncate_history_exact_limit() {
    let (mut session, _client) = setup_session().await;
    session.max_history = 5;
    session.chat_history = (0..5_u32)
        .map(|i| Message {
            role: "user".to_string(),
            content: format!("message {}", i),
        })
        .collect();

    session.truncate_history();

    assert_eq!(session.chat_history.len(), 5);
}

// ---------------------------------------------------------------------------
// New integration tests
// ---------------------------------------------------------------------------

/// Verify a complete session lifecycle: chat.start → chat.message → chat.stop,
/// checking that every step produces the expected server response sequence.
#[tokio::test]
async fn test_full_session_lifecycle() {
    let (session, client, shutdown_tx) = setup_session_with_shutdown_tx().await;

    // Spawn the session so it processes messages concurrently.
    let session_handle = tokio::spawn(session.run());

    // Keep shutdown_tx alive to prevent premature shutdown
    let _shutdown_tx_guard = shutdown_tx;

    // Split the client into owned read/write halves.
    let (reader_half, mut writer_half) = client.into_split();
    let mut reader = tokio::io::BufReader::new(reader_half);

    // Step 1 — chat.start
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.start","agent_id":"my-agent","id":"req-start"}"#,
    )
    .await;
    let msg1 = read_server_message(&mut reader).await;
    match msg1 {
        ServerMessage::ChatStarted { session_id, id } => {
            assert_eq!(session_id, "test-session");
            assert_eq!(id, "req-start");
        }
        other => panic!("expected ChatStarted, got {other:?}"),
    }

    // Step 2 — chat.message (StubProvider responds "stub response")
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"hello world","id":"req-msg"}"#,
    )
    .await;
    let msg2a = read_server_message(&mut reader).await;
    let msg2b = read_server_message(&mut reader).await;
    match (&msg2a, &msg2b) {
        (
            ServerMessage::ChatResponse { content, done, id },
            ServerMessage::ChatResponseDone { id: id2 },
        ) => {
            assert_eq!(*content, "stub response");
            assert!(done);
            assert_eq!(id, "req-msg");
            assert_eq!(id2, "req-msg");
        }
        _ => panic!("expected ChatResponse + ChatResponseDone, got {msg2a:?} + {msg2b:?}"),
    }

    // Step 3 — chat.stop
    send_client_json(&mut writer_half, r#"{"type":"chat.stop","id":"req-stop"}"#).await;
    let msg3 = read_server_message(&mut reader).await;
    match msg3 {
        ServerMessage::ChatResponseDone { id } => {
            assert_eq!(id, "req-stop");
        }
        other => panic!("expected ChatResponseDone, got {other:?}"),
    }

    // Clean up.
    drop(writer_half);
    let _ = session_handle.await;
}

/// Verify that when the server shutdown signal is fired, the session responds
/// with a `ChatError` carrying the message "server shutting down".
#[tokio::test]
async fn test_session_shutdown_signal() {
    std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(2);
    let registry = Arc::new(LLMRegistry::new());
    registry
        .register("stub".to_string(), Arc::new(StubProvider::new()))
        .await;
    let session = ChatSession::new(
        "shutdown-session".to_string(),
        "shutdown-agent".to_string(),
        accepted,
        shutdown_rx,
        registry,
    );
    std::env::remove_var("LLM_FALLBACK_CHAIN");

    // Spawn session and immediately send the shutdown signal by dropping the tx.
    let session_handle = tokio::spawn(session.run());
    drop(shutdown_tx);

    // Read the error message from the client.
    let (reader_half, _) = client.into_split();
    let mut reader = tokio::io::BufReader::new(reader_half);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let msg: ServerMessage = serde_json::from_str(line.trim()).unwrap();

    match msg {
        ServerMessage::ChatError { message, .. } => {
            assert!(
                message.contains("server shutting down"),
                "expected 'server shutting down', got: {message}"
            );
        }
        other => panic!("expected ChatError, got {other:?}"),
    }

    let _ = session_handle.await;
}
