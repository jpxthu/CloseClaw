#![cfg(feature = "chat-legacy")]

//! E2E tests for session compaction via /compact command.
//!
//! Verifies the complete integration path:
//! multi-round conversation → /compact → LLM summary → boundary message → session continues.
//!
//! Run with: `cargo test --features fake-llm --test e2e_session_compact_tests`

#![cfg(feature = "fake-llm")]

use closeclaw::chat::protocol::ServerMessage;
use closeclaw::chat::session::LegacyChatSession;
use closeclaw::llm::provider::Provider;
use closeclaw::llm::LLMRegistry;
use std::env::{remove_var, set_var, var};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Set up a `LegacyChatSession` backed by a TCP pair with a FakeProvider
/// registered in the LLMRegistry.
async fn setup_session_with_fake(
    scenarios: Vec<(String, String)>,
) -> (
    LegacyChatSession,
    tokio::net::TcpStream,
    broadcast::Sender<()>,
) {
    use closeclaw::llm::fake::FakeProvider;

    let mut builder = FakeProvider::builder();
    for (content, model) in scenarios {
        builder = builder.then_ok(content, model);
    }
    let old_val = var("LLM_FALLBACK_CHAIN").ok();
    set_var("LLM_FALLBACK_CHAIN", "fake/glm-5");
    let fake_provider = builder.or_else("fallback response").build();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(2);

    let registry = Arc::new(LLMRegistry::new());
    let wrapped: Arc<dyn Provider> = Arc::new(fake_provider);
    registry.register("fake".to_string(), wrapped).await;

    let session = LegacyChatSession::new(
        "test-session".to_string(),
        "test-agent".to_string(),
        accepted,
        shutdown_rx,
        registry,
        None, // config_dir
    );
    match old_val {
        Some(v) => set_var("LLM_FALLBACK_CHAIN", v),
        None => remove_var("LLM_FALLBACK_CHAIN"),
    }

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

/// Send a raw JSON client message.
async fn send_client_json(writer: &mut tokio::net::tcp::OwnedWriteHalf, json: &str) {
    writer.write_all(json.as_bytes()).await.unwrap();
    writer.write_all(b"\n").await.unwrap();
    writer.flush().await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test: 3 rounds of conversation → /compact → boundary message replaces history
/// → continue conversation → second /compact.
#[ignore = "chat module removed in #725; rewrite against new interface (see new issue)"]
#[tokio::test]
async fn test_session_compact_e2e() {
    let scenarios = vec![
        ("reply 1".to_string(), "glm-5".to_string()),
        ("reply 2".to_string(), "glm-5".to_string()),
        ("reply 3".to_string(), "glm-5".to_string()),
        (
            "<summary>User and assistant discussed various topics.\n[boundary]</summary>"
                .to_string(),
            "glm-5".to_string(),
        ),
        ("reply after compact".to_string(), "glm-5".to_string()),
        (
            "<summary>Second summary.\n[boundary]</summary>".to_string(),
            "glm-5".to_string(),
        ),
    ];

    let (session, client, shutdown_tx) = setup_session_with_fake(scenarios).await;

    let session_handle = tokio::spawn(session.run());
    let _shutdown_guard = shutdown_tx;

    let (reader_half, mut writer_half) = client.into_split();
    let mut reader = tokio::io::BufReader::new(reader_half);

    // Start session
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.start","agent_id":"my-agent","id":"req-start"}"#,
    )
    .await;
    let start_msg = read_server_message(&mut reader).await;
    assert!(
        matches!(start_msg, ServerMessage::ChatStarted { .. }),
        "expected ChatStarted, got {start_msg:?}"
    );

    // Round 1: user → assistant
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"hello","id":"req-msg-1"}"#,
    )
    .await;
    let resp1a = read_server_message(&mut reader).await;
    let resp1b = read_server_message(&mut reader).await;
    assert!(
        matches!(
            (&resp1a, &resp1b),
            (ServerMessage::ChatResponse { content, .. }, ServerMessage::ChatResponseDone { .. })
                if content == "reply 1"
        ),
        "round 1: expected 'reply 1', got resp1a={resp1a:?}, resp1b={resp1b:?}"
    );

    // Round 2
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"how are you","id":"req-msg-2"}"#,
    )
    .await;
    let resp2a = read_server_message(&mut reader).await;
    let resp2b = read_server_message(&mut reader).await;
    assert!(
        matches!(
            (&resp2a, &resp2b),
            (ServerMessage::ChatResponse { content, .. }, ServerMessage::ChatResponseDone { .. })
                if content == "reply 2"
        ),
        "round 2: expected 'reply 2', got resp2a={resp2a:?}, resp2b={resp2b:?}"
    );

    // Round 3
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"tell me more","id":"req-msg-3"}"#,
    )
    .await;
    let resp3a = read_server_message(&mut reader).await;
    let resp3b = read_server_message(&mut reader).await;
    assert!(
        matches!(
            (&resp3a, &resp3b),
            (ServerMessage::ChatResponse { content, .. }, ServerMessage::ChatResponseDone { .. })
                if content == "reply 3"
        ),
        "round 3: expected 'reply 3', got resp3a={resp3a:?}, resp3b={resp3b:?}"
    );

    // Step 3 — Trigger /compact
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"/compact","id":"req-compact-1"}"#,
    )
    .await;

    // Expect: success message (contains "压缩成功") + ChatResponseDone
    let compact1a = read_server_message(&mut reader).await;
    let compact1b = read_server_message(&mut reader).await;
    let compact1_content = match &compact1a {
        ServerMessage::ChatResponse { content, .. } => content.clone(),
        other => panic!("expected ChatResponse for /compact, got {other:?}"),
    };
    assert!(
        compact1_content.contains("压缩成功"),
        "/compact should report success, got: {compact1_content}"
    );
    assert!(
        matches!(compact1b, ServerMessage::ChatResponseDone { .. }),
        "expected ChatResponseDone after /compact, got {compact1b:?}"
    );

    // Step 4 — Continue conversation after compact
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"continue","id":"req-msg-4"}"#,
    )
    .await;
    let resp4a = read_server_message(&mut reader).await;
    let resp4b = read_server_message(&mut reader).await;
    // After /compact: round 4 — uses scenario 5
    assert!(
        matches!(
            (&resp4a, &resp4b),
            (ServerMessage::ChatResponse { content, .. }, ServerMessage::ChatResponseDone { .. })
                if content == "reply after compact"
        ),
        "round 4 after compact: expected 'reply after compact', \
         got resp4a={resp4a:?}, resp4b={resp4b:?}"
    );

    // Step 5 — Second /compact
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"/compact","id":"req-compact-2"}"#,
    )
    .await;
    let compact2a = read_server_message(&mut reader).await;
    let compact2b = read_server_message(&mut reader).await;
    let compact2_content = match &compact2a {
        ServerMessage::ChatResponse { content, .. } => content.clone(),
        other => panic!("expected ChatResponse for /compact, got {other:?}"),
    };
    assert!(
        compact2_content.contains("压缩成功"),
        "second /compact should report success, got: {compact2_content}"
    );
    assert!(
        matches!(compact2b, ServerMessage::ChatResponseDone { .. }),
        "expected ChatResponseDone for second /compact, got {compact2b:?}"
    );

    // Clean up
    drop(writer_half);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session_handle).await;
}

/// Test: /compact with extra arguments returns error.
#[ignore = "chat module removed in #725; rewrite against new interface (see new issue)"]
#[tokio::test]
async fn test_compact_invalid_syntax_returns_error() {
    let scenarios = vec![(
        "<summary>summary text</summary>".to_string(),
        "glm-5".to_string(),
    )];

    let (session, client, shutdown_tx) = setup_session_with_fake(scenarios).await;

    let session_handle = tokio::spawn(session.run());
    let _shutdown_guard = shutdown_tx;

    let (reader_half, mut writer_half) = client.into_split();
    let mut reader = tokio::io::BufReader::new(reader_half);

    // Start session
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.start","agent_id":"my-agent","id":"req-start"}"#,
    )
    .await;
    let _start_msg = read_server_message(&mut reader).await;

    // Send /compact with invalid syntax (e.g., /compact extra-arg not supported)
    send_client_json(
        &mut writer_half,
        r#"{"type":"chat.message","content":"/compact extra arg","id":"req-invalid"}"#,
    )
    .await;

    let err_a = read_server_message(&mut reader).await;
    let err_b = read_server_message(&mut reader).await;
    let err_content = match &err_a {
        ServerMessage::ChatResponse { content, .. } => content.clone(),
        other => panic!("expected ChatResponse for invalid /compact, got {other:?}"),
    };
    assert!(
        err_content.contains("[error]"),
        "invalid /compact should return error, got: {err_content}"
    );
    assert!(
        matches!(err_b, ServerMessage::ChatResponseDone { .. }),
        "expected ChatResponseDone after error, got {err_b:?}"
    );

    drop(writer_half);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session_handle).await;
}
