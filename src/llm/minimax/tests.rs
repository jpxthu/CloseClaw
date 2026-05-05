//! Unit tests for the MiniMax provider.

use super::*;

// --- Fixture-based deserialization and content extraction tests ---

#[test]
fn test_simple_chat_deserialize_and_extract() {
    let json = include_str!("../../tests/fixtures/llm/minimax/simple-chat.json");
    let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
    let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = MiniMaxProvider::extract_content(msg);
    assert!(
        !extracted.is_empty(),
        "Expected non-empty extracted content from reasoning_content"
    );
    assert_eq!(extracted, msg.reasoning_content.as_ref().unwrap().trim());
}

#[test]
fn test_simple_chat_content_priority_over_reasoning() {
    // both-content.json: both content and reasoning_content populated → content wins
    let json = include_str!("../../tests/fixtures/llm/minimax/both-content.json");
    let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
    let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = MiniMaxProvider::extract_content(msg);
    // content takes priority
    assert!(extracted.starts_with("Final answer:"));
}

#[test]
fn test_models_list_deserialize() {
    let json = include_str!("../../tests/fixtures/llm/minimax/models-list.json");
    let resp: MiniMaxModelsResponse = serde_json::from_str(json).unwrap();
    assert!(!resp.models.is_empty());
    // At least one model should have reasoning enabled
    let has_reasoning = resp.models.iter().any(|m| {
        m.usage
            .completion_tokens_details
            .as_ref()
            .map_or(false, |d| d.reasoning_tokens > 0)
    });
    assert!(has_reasoning, "Expected at least one reasoning model");
}

// --- Integration tests via mockito ---

#[tokio::test]
async fn test_chat_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/v1/text/chatcompletion_pro")
        .match_header("Authorization", mockito::Matcher::Regex(r"Bearer .+".to_string()))
        .match_header("Content-Type", "application/json")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"choices":[{"message":{"role":"assistant","content":"hi"}},"usage":{"completion_tokens":10,"prompt_tokens":5}}"#)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let provider = MiniMaxProvider::with_base_url("test-key".into(), server.url(), client);
    let req = create_chat_request("Abab5.5-chat");
    let result = provider.chat(req).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let content = &result.unwrap().message;
    assert!(content.contains("hi"));
}

#[tokio::test]
async fn test_chat_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/v1/text/chatcompletion_pro")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"error_code":1007,"error_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let provider = MiniMaxProvider::with_base_url("test-key".into(), server.url(), client);
    let err = provider
        .chat(create_chat_request("Abab5.5-chat"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::AuthFailed(_));
}

#[tokio::test]
async fn test_chat_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/v1/text/chatcompletion_pro")
        .match_body(mockito::Matcher::Any)
        .with_status(429)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"error_code":2001,"error_msg":"rate limit"}}"#)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let provider = MiniMaxProvider::with_base_url("test-key".into(), server.url(), client);
    let err = provider
        .chat(create_chat_request("Abab5.5-chat"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::RateLimitExceeded);
}

#[tokio::test]
async fn test_fetch_model_list_success_mock() {
    let fixture = include_str!("../../tests/fixtures/llm/minimax/models-list.json");
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/v1/models")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let provider = MiniMaxProvider::with_base_url("test-key".into(), server.url(), client);
    let models = provider.fetch_model_list("test-key").await.unwrap();

    m.assert_async().await;
    assert!(!models.is_empty());
    // Verified reasoning models are filtered in
    let has_reasoning = models.iter().any(|m| {
        m.usage
            .completion_tokens_details
            .as_ref()
            .map_or(false, |d| d.reasoning_tokens > 0)
    });
    assert!(has_reasoning);
}

// --- Helper functions ---

fn create_chat_request(model: &str) -> LLMRequest {
    LLMRequest {
        model: model.to_string(),
        messages: vec![LLMMessage::user("hi")],
        temperature: None,
        top_p: None,
        max_tokens: None,
        stream: false,
        stop: None,
    }
}
