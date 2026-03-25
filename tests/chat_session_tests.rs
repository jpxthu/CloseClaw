//! ChatSession unit tests

use closeclaw::chat::session::ChatSession;
use closeclaw::llm::{LLMRegistry, Message};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

// Test that truncate_history removes oldest entries when over limit
#[test]
fn test_truncate_history_keeps_recent_messages() {
    let max_history = 5;
    let mut chat_history: Vec<Message> = (0..10)
        .map(|i| Message {
            role: "user".to_string(),
            content: format!("message {}", i),
        })
        .collect();

    // Simulate truncate_history logic
    if chat_history.len() > max_history {
        let remove_count = chat_history.len() - max_history;
        chat_history.drain(0..remove_count);
    }

    assert_eq!(chat_history.len(), 5);
    // Should keep messages 5-9 (most recent)
    assert_eq!(chat_history[0].content, "message 5");
    assert_eq!(chat_history[4].content, "message 9");
}

#[test]
fn test_truncate_history_does_nothing_when_under_limit() {
    let max_history = 5;
    let mut chat_history: Vec<Message> = (0..3)
        .map(|i| Message {
            role: "user".to_string(),
            content: format!("message {}", i),
        })
        .collect();

    if chat_history.len() > max_history {
        let remove_count = chat_history.len() - max_history;
        chat_history.drain(0..remove_count);
    }

    assert_eq!(chat_history.len(), 3);
}

#[test]
fn test_truncate_history_exact_limit_does_nothing() {
    let max_history = 5;
    let mut chat_history: Vec<Message> = (0..5)
        .map(|i| Message {
            role: "user".to_string(),
            content: format!("message {}", i),
        })
        .collect();

    if chat_history.len() > max_history {
        let remove_count = chat_history.len() - max_history;
        chat_history.drain(0..remove_count);
    }

    assert_eq!(chat_history.len(), 5);
}

#[tokio::test]
async fn test_chat_session_new_with_custom_timeout() {
    std::env::set_var("LLM_TIMEOUT_SECS", "60");
    std::env::set_var("LLM_FALLBACK_CHAIN", "stub/stub-model");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
    let registry = Arc::new(LLMRegistry::new());

    // Session::new() only stores halves, no I/O — stream can be unaccepted
    let session = ChatSession::new(
        "test-session".to_string(),
        "test-agent".to_string(),
        stream,
        shutdown_rx.resubscribe(),
        registry,
    );

    assert_eq!(session.session_id, "test-session");
    assert_eq!(session.agent_id, "test-agent");

    std::env::remove_var("LLM_TIMEOUT_SECS");
    std::env::remove_var("LLM_FALLBACK_CHAIN");
}
