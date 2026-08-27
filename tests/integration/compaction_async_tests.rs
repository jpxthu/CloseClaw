//! Async integration tests for the session compaction service.
//!
//! Verifies `CompactionService::compact` against the injected-`ChatFn` API:
//! success, empty input, LLM failure, missing summary, custom instructions,
//! auto trigger, and system-prompt / message routing.
//!
//! Uses a mock `ChatFn` (not `FakeProvider`), so these tests do not require
//! the `fake-llm` feature.

use std::sync::Arc;

use closeclaw_session::compaction::{
    ChatFn, CompactConfig, CompactionError, CompactionMessage, CompactionService,
};

/// Build a [`ChatFn`] that returns a successful `<summary>` response.
fn mock_chat_success(summary: &str) -> ChatFn {
    let response = format!("<summary>{}</summary>", summary);
    Arc::new(move |_model: String, _msgs: Vec<CompactionMessage>| {
        let resp = response.clone();
        Box::pin(async move { Ok((resp, 0)) })
    })
}

/// Build a [`ChatFn`] that simulates an LLM call failure.
fn mock_chat_failure(error_msg: &str) -> ChatFn {
    let err = error_msg.to_string();
    Arc::new(move |_model: String, _msgs: Vec<CompactionMessage>| {
        let e = err.clone();
        Box::pin(async move { Err(e) })
    })
}

/// Build a [`ChatFn`] that returns a response without `<summary>` tags.
fn mock_chat_no_summary(response: &str) -> ChatFn {
    let resp = response.to_string();
    Arc::new(move |_model: String, _msgs: Vec<CompactionMessage>| {
        let r = resp.clone();
        Box::pin(async move { Ok((r, 0)) })
    })
}

/// A representative two-turn conversation.
fn sample_messages() -> Vec<CompactionMessage> {
    vec![
        CompactionMessage {
            role: "user".to_string(),
            content: "Hello, this is a test message".to_string(),
        },
        CompactionMessage {
            role: "assistant".to_string(),
            content: "Hi! How can I help you?".to_string(),
        },
    ]
}

#[tokio::test]
async fn test_compact_success() {
    let mut svc = CompactionService::new(CompactConfig::default());
    let chat_fn = mock_chat_success("Compacted summary content");

    let result = svc
        .compact(&sample_messages(), "glm-5.1", None, false, None, &chat_fn)
        .await
        .unwrap();

    assert!(result.performed);
    assert!(result
        .boundary_message
        .contains("Compacted summary content"));
    assert!(result.boundary_message.contains("手动压缩"));
}

#[tokio::test]
async fn test_compact_empty_messages() {
    let mut svc = CompactionService::new(CompactConfig::default());
    let chat_fn = mock_chat_success("content");

    let result = svc
        .compact(&[], "glm-5.1", None, true, None, &chat_fn)
        .await;

    assert!(matches!(result, Err(CompactionError::EmptyMessages)));
}

#[tokio::test]
async fn test_compact_llm_failure() {
    let mut svc = CompactionService::new(CompactConfig::default());
    let chat_fn = mock_chat_failure("rate limit exceeded");

    let result = svc
        .compact(&sample_messages(), "glm-5.1", None, false, None, &chat_fn)
        .await;

    assert!(matches!(result, Err(CompactionError::LLMCallFailed(_))));
}

#[tokio::test]
async fn test_compact_no_summary() {
    let mut svc = CompactionService::new(CompactConfig::default());
    let chat_fn = mock_chat_no_summary("No summary tag in response");

    let result = svc
        .compact(&sample_messages(), "glm-5.1", None, true, None, &chat_fn)
        .await;

    assert!(matches!(result, Err(CompactionError::SummaryParseFailed)));
}

#[tokio::test]
async fn test_compact_with_custom_instructions() {
    let mut svc = CompactionService::new(CompactConfig::default());
    let chat_fn = mock_chat_success("Test summary");

    let result = svc
        .compact(
            &sample_messages(),
            "glm-5.1",
            Some("重点保留用户名"),
            true,
            None,
            &chat_fn,
        )
        .await
        .unwrap();

    assert!(result.boundary_message.contains("Test summary"));
    assert!(result.boundary_message.contains("自动压缩"));
}

#[tokio::test]
async fn test_compact_auto_trigger() {
    let mut svc = CompactionService::new(CompactConfig::default());
    let chat_fn = mock_chat_success("Auto summary");

    let result = svc
        .compact(&sample_messages(), "glm-5.1", None, true, None, &chat_fn)
        .await
        .unwrap();

    assert!(result.is_auto);
}

#[tokio::test]
async fn test_compact_prepends_system_prompt_and_passes_messages() {
    use std::sync::Mutex;

    let mut svc = CompactionService::new(CompactConfig::default());

    let captured: Arc<Mutex<Vec<Vec<CompactionMessage>>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);
    let chat_fn: ChatFn = Arc::new(move |_model: String, msgs: Vec<CompactionMessage>| {
        let cap = Arc::clone(&captured_clone);
        Box::pin(async move {
            cap.lock().unwrap().push(msgs);
            Ok(("<summary>Filtered summary</summary>".to_string(), 0))
        })
    });

    let messages = vec![
        CompactionMessage {
            role: "system".to_string(),
            content: "You are a helpful assistant.".to_string(),
        },
        CompactionMessage {
            role: "user".to_string(),
            content: "Hello from user".to_string(),
        },
        CompactionMessage {
            role: "system".to_string(),
            content: "Another system instruction.".to_string(),
        },
        CompactionMessage {
            role: "assistant".to_string(),
            content: "Hello from assistant".to_string(),
        },
    ];

    let result = svc
        .compact(&messages, "glm-5.1", None, false, None, &chat_fn)
        .await
        .unwrap();
    assert!(result.performed);

    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 1, "chat_fn should be called exactly once");
    let req = &captured[0];

    // First message is the compaction system prompt.
    assert_eq!(req[0].role, "system");
    assert!(req[0].content.contains("session summarizer"));

    // The 4 input messages are passed through unchanged (in order).
    assert_eq!(req.len(), 5, "1 system prompt + 4 input messages");
    assert_eq!(req[1].role, "system");
    assert_eq!(req[1].content, "You are a helpful assistant.");
    assert_eq!(req[2].role, "user");
    assert_eq!(req[2].content, "Hello from user");
    assert_eq!(req[3].role, "system");
    assert_eq!(req[3].content, "Another system instruction.");
    assert_eq!(req[4].role, "assistant");
    assert_eq!(req[4].content, "Hello from assistant");
}
