//! Unit tests for the MiniMax provider.

use super::*;
use crate::llm::Message;

// --- Fixture-based deserialization and content extraction tests ---

#[test]
fn test_simple_chat_deserialize_and_extract() {
    let json = include_str!("../../../tests/fixtures/llm/minimax/simple-chat.json");
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
    let json = include_str!("../../../tests/fixtures/llm/minimax/both-content.json");
    let resp: MiniMaxResponse = serde_json::from_str(json).unwrap();
    let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = MiniMaxProvider::extract_content(msg);
    // content takes priority
    assert!(extracted.starts_with("Final answer:"));
}

#[test]
fn test_models_list_deserialize() {
    let json = include_str!("../../../tests/fixtures/llm/minimax/models-list.json");
    let resp: MiniMaxModelsResponse = serde_json::from_str(json).unwrap();
    assert!(!resp.data.is_empty());
    assert_eq!(resp.data.len(), 4);
}

// --- Integration tests via mockito ---

fn mock_provider(server: &mockito::Server) -> MiniMaxProvider {
    MiniMaxProvider {
        api_key: "test-key".into(),
        base_url: server.url(),
        http_client: reqwest::Client::new(),
    }
}

#[tokio::test]
async fn test_chat_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .match_header("Content-Type", "application/json")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(
            r#"{
            "choices":[{"message":{"role":"assistant","content":"hi"}}],
            "usage":{"completion_tokens":10,"prompt_tokens":5}
        }"#,
        )
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_chat_request("Abab5.5-chat");
    let result = provider.chat(req).await;

    m.assert_async().await;
    assert!(result.is_ok());
    assert!(result.unwrap().content.contains("hi"));
}

#[tokio::test]
async fn test_chat_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"error_code":1007,"error_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let err = provider
        .chat(create_chat_request("Abab5.5-chat"))
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, LLMError::AuthFailed(_)));
}

#[tokio::test]
async fn test_chat_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Any)
        .with_status(429)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"error_code":2001,"error_msg":"rate limit"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let err = provider
        .chat(create_chat_request("Abab5.5-chat"))
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, LLMError::RateLimitExceeded));
}

#[tokio::test]
async fn test_fetch_model_list_success_mock() {
    let fixture = include_str!("../../../tests/fixtures/llm/minimax/models-list.json");
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

    let provider = mock_provider(&server);
    let models = provider.fetch_model_list("test-key").await.unwrap();

    m.assert_async().await;
    assert!(!models.is_empty());
    // Verify model IDs are correctly parsed from the fixture
    let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"MiniMax-M2"));
    assert!(ids.contains(&"MiniMax-M2.7"));
}

#[tokio::test]
async fn test_fetch_model_list_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/v1/models")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1007,"status_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let err = provider.fetch_model_list("test-key").await.unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, LLMError::AuthFailed(_)));
}

#[tokio::test]
async fn test_fetch_model_list_timeout_mock() {
    // Use an http_client with a very short reqwest timeout (1ms) so the
    // HTTP send returns an error, which gets mapped to LLMError::NetworkError.
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/v1/models")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body_from_fn(|_w| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            Ok(())
        })
        .create_async()
        .await;

    let provider = MiniMaxProvider {
        api_key: "test-key".into(),
        base_url: server.url(),
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1))
            .build()
            .unwrap(),
    };

    let err = provider.fetch_model_list("test-key").await.unwrap_err();

    assert!(matches!(err, LLMError::NetworkError(_)));
}

// --- Helper functions ---

fn create_chat_request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        temperature: 0.7,
        max_tokens: None,
    }
}
