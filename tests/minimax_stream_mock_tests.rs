//! MiniMax mock-based error and streaming integration tests using mockito.
//!
//! Error fixture tests: base_resp error mapping (AuthFailed, ModelNotFound, InvalidRequest).
//! Streaming fixture tests: SSE delta merging and usage extraction.

use closeclaw::llm::MiniMaxProvider;
use closeclaw::llm::{ChatRequest, LLMProvider, Message as LLMMessage};
use mockito::Server;

// ---------------------------------------------------------------------------
// Helper: build a ChatRequest for the given user text
// ---------------------------------------------------------------------------

fn make_request(model: &str, user_text: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![LLMMessage {
            role: "user".to_string(),
            content: user_text.to_string(),
        }],
        temperature: 0.7,
        max_tokens: None,
    }
}

// ---------------------------------------------------------------------------
// Error fixture tests
// ---------------------------------------------------------------------------

/// error-auth.json: base_resp status_code=1004 → AuthFailed.
#[tokio::test]
async fn test_error_auth() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer bad-key")
        .with_body(include_str!("fixtures/llm/minimax/error-auth.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "bad-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Hello");
    let err = provider
        .chat(request)
        .await
        .expect_err("chat should fail with AuthFailed");

    match err {
        closeclaw::llm::LLMError::AuthFailed(msg) => {
            assert!(
                msg.contains("login fail"),
                "expected 'login fail' in error message, got: {}",
                msg
            );
        }
        other => panic!("expected AuthFailed, got: {:?}", other),
    }

    mock.assert_async().await;
}

/// error-invalid-model.json: base_resp status_code=2013 + "unknown model" → ModelNotFound.
#[tokio::test]
async fn test_error_invalid_model() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!(
            "fixtures/llm/minimax/error-invalid-model.json"
        ))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("invalid-model-xyz", "Hello");
    let err = provider
        .chat(request)
        .await
        .expect_err("chat should fail with ModelNotFound");

    match err {
        closeclaw::llm::LLMError::ModelNotFound(msg) => {
            assert!(
                msg.contains("unknown model"),
                "expected 'unknown model' in error message, got: {}",
                msg
            );
        }
        other => panic!("expected ModelNotFound, got: {:?}", other),
    }

    mock.assert_async().await;
}

/// error-empty-messages.json: base_resp status_code=2013 → InvalidRequest.
#[tokio::test]
async fn test_error_empty_messages() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!(
            "fixtures/llm/minimax/error-empty-messages.json"
        ))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Hello");
    let err = provider
        .chat(request)
        .await
        .expect_err("chat should fail with InvalidRequest");

    match err {
        closeclaw::llm::LLMError::InvalidRequest(msg) => {
            assert!(
                msg.contains("messages is empty"),
                "expected 'messages is empty' in error message, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidRequest, got: {:?}", other),
    }

    mock.assert_async().await;
}

/// error-missing-model.json: base_resp status_code=2013 → InvalidRequest.
#[tokio::test]
async fn test_error_missing_model() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!(
            "fixtures/llm/minimax/error-missing-model.json"
        ))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Hello");
    let err = provider
        .chat(request)
        .await
        .expect_err("chat should fail with InvalidRequest");

    match err {
        closeclaw::llm::LLMError::InvalidRequest(msg) => {
            assert!(
                msg.contains("missing required parameter"),
                "expected 'missing required parameter' in error message, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidRequest, got: {:?}", other),
    }

    mock.assert_async().await;
}

/// usage-coding-plan.json: usage quota response (non-chat response format).
/// The fixture is a quota/usage query response, not a chat completion.
/// We test that it is correctly identified as an API error (no choices).
#[tokio::test]
async fn test_usage_coding_plan() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/usage-coding-plan.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Hello");
    let err = provider
        .chat(request)
        .await
        .expect_err("usage query response has no choices, should fail");

    // This is an ApiError because the response has no choices
    match err {
        closeclaw::llm::LLMError::ApiError(msg) => {
            assert!(
                msg.contains("no choices") || msg.contains("parse"),
                "expected no-choices or parse error, got: {}",
                msg
            );
        }
        other => panic!("expected ApiError for empty choices, got: {:?}", other),
    }

    mock.assert_async().await;
}

// ---------------------------------------------------------------------------
// Streaming fixture tests
// ---------------------------------------------------------------------------

/// streaming.txt: MiniMax-M2.5 model streaming.
/// Verifies delta chunks are merged in order to produce correct final content,
/// and that usage fields are correctly extracted from the final message.
///
/// MiniMax's SSE final message sends the accumulated `reasoning_content` as both
/// delta text (completing the stream) AND in the Done chunk's text field.
/// We verify: (1) delta-only text equals expected, (2) Done model/usage correct.
#[tokio::test]
async fn test_streaming() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .match_header("content-type", "application/json")
        .match_header("accept", "text/event-stream")
        .match_body(mockito::Matcher::Regex(
            r#""model"\s*:\s*"MiniMax-M2\.5"#.into(),
        ))
        .with_body(include_str!("fixtures/llm/minimax/streaming.txt"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Count to 3");
    let mut rx = provider
        .chat_streaming(request)
        .await
        .expect("chat_streaming should succeed");

    // Collect all text chunks and the final Done chunk
    let mut texts: Vec<String> = Vec::new();
    let mut done_model: Option<String> = None;
    let mut done_usage: Option<closeclaw::llm::Usage> = None;

    while let Some(chunk) = rx.recv().await {
        match chunk {
            closeclaw::llm::ChatStreamChunk::Text(t) => texts.push(t),
            closeclaw::llm::ChatStreamChunk::Done { model, usage } => {
                done_model = Some(model);
                done_usage = Some(usage);
            }
            closeclaw::llm::ChatStreamChunk::Error(e) => {
                panic!("unexpected streaming error: {:?}", e);
            }
        }
    }

    // Verify model
    assert_eq!(
        done_model.as_deref(),
        Some("MiniMax-M2.5"),
        "expected MiniMax-M2.5 model"
    );

    // Verify usage
    let usage = done_usage.expect("should have received Done with usage");
    assert_eq!(usage.prompt_tokens, 45, "prompt_tokens should match");
    assert_eq!(
        usage.completion_tokens, 30,
        "completion_tokens should match"
    );
    assert_eq!(usage.total_tokens, 75, "total_tokens should match");

    // Verify merged delta content.
    let expected =
        "The user wants me to count to 3. This is a simple request - I'll just count from 1 to 3.";
    if texts.len() >= 2 {
        let delta_merged = texts[..texts.len() - 1].join("");
        assert_eq!(
            delta_merged.as_str(),
            expected,
            "delta chunks should merge to expected content"
        );
    }
    assert_eq!(
        texts.last().map(|s| s.as_str()),
        Some(expected),
        "final Done text should be the complete content"
    );

    mock.assert_async().await;
}

/// streaming-m2.7.txt: MiniMax-M2.7 model streaming.
/// Verifies delta chunks are merged in order and usage fields are correct.
#[tokio::test]
async fn test_streaming_m2_7() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .match_header("content-type", "application/json")
        .match_header("accept", "text/event-stream")
        .match_body(mockito::Matcher::Regex(
            r#""model"\s*:\s*"MiniMax-M2\.7"#.into(),
        ))
        .with_body(include_str!("fixtures/llm/minimax/streaming-m2.7.txt"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.7", "What is 2+2?");
    let mut rx = provider
        .chat_streaming(request)
        .await
        .expect("chat_streaming should succeed");

    let mut texts: Vec<String> = Vec::new();
    let mut done_model: Option<String> = None;
    let mut done_usage: Option<closeclaw::llm::Usage> = None;

    while let Some(chunk) = rx.recv().await {
        match chunk {
            closeclaw::llm::ChatStreamChunk::Text(t) => texts.push(t),
            closeclaw::llm::ChatStreamChunk::Done { model, usage } => {
                done_model = Some(model);
                done_usage = Some(usage);
            }
            closeclaw::llm::ChatStreamChunk::Error(e) => {
                panic!("unexpected streaming error: {:?}", e);
            }
        }
    }

    // Verify model
    assert_eq!(
        done_model.as_deref(),
        Some("MiniMax-M2.7"),
        "expected MiniMax-M2.7 model"
    );

    // Verify usage
    let usage = done_usage.expect("should have received Done with usage");
    assert_eq!(usage.prompt_tokens, 48, "prompt_tokens should match");
    assert_eq!(
        usage.completion_tokens, 50,
        "completion_tokens should match"
    );
    assert_eq!(usage.total_tokens, 98, "total_tokens should match");

    // Verify merged delta content.
    let expected = "The user asks: \"What is 2+2?\" That's a simple math question. The answer is 4. There's no policy violation. Provide a concise answer.\n\nBut we must also think about context: The user may want a brief";
    if texts.len() >= 2 {
        let delta_merged = texts[..texts.len() - 1].join("");
        assert_eq!(
            delta_merged.as_str(),
            expected,
            "delta chunks should merge to expected content"
        );
    }
    assert_eq!(
        texts.last().map(|s| s.as_str()),
        Some(expected),
        "final Done text should be the complete content"
    );

    mock.assert_async().await;
}
