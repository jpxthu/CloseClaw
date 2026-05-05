//! Unit tests for the DeepSeek provider.

use super::*;
use mockito::Server;

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn provider_url(server: &mockito::Server) -> String {
    format!("http://{}", server.address())
}

fn chat_request(model: &str) -> LLMRequest {
    LLMRequest {
        model: model.to_string(),
        messages: vec![LLMMessage::user("Hello")],
        temperature: None,
        top_p: None,
        max_tokens: None,
        stream: false,
        stop: None,
    }
}

// ---------------------------------------------------------------------------
// Chat integration tests via mockito
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/chat-success.json");

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer sk-.*".to_string()),
        )
        .match_header("content-type", "application/json")
        .match_header("accept", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let result = provider.chat(chat_request("deepseek-v4-flash")).await;

    m.assert_async().await;
    assert!(result.is_ok(), "expected ok, got {:?}", result);
}

#[tokio::test]
async fn test_chat_error_not_found_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/error-not-found.json");

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .match_header("content-type", "application/json")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("unknown-model"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::ModelNotFound(_));
}

#[tokio::test]
async fn test_chat_error_auth_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/error-auth.json");

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .match_header("content-type", "application/json")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("deepseek-v4-flash"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::AuthFailed(_));
}

#[tokio::test]
async fn test_chat_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(429)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"rate_limit_exceeded","message":"rate limit exceeded"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("deepseek-v4-flash"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::RateLimitExceeded);
}

// ---------------------------------------------------------------------------
// fetch_model_list() tests via mockito
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fetch_model_list_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/models-list.json");

    let m = server
        .mock("GET", "/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), server.url());
    let models = provider.fetch_model_list("fake-key").await.unwrap();

    m.assert_async().await;
    assert!(!models.is_empty(), "expected at least one model");
    // Verify deprecated models are filtered out
    for model in &models {
        assert!(
            !model.name.to_lowercase().contains("deprecated"),
            "Deprecated model {} should be filtered",
            model.name
        );
    }
}

#[tokio::test]
async fn test_fetch_model_list_http_error_mock() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("GET", "/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"invalid_api_key","message":"invalid api key"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), server.url());
    let err = provider.fetch_model_list("fake-key").await.unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::AuthFailed(_));
}

// -------------------------------------------------------------------------
// extract_content() unit tests
// -------------------------------------------------------------------------

#[test]
fn test_extract_content_with_content() {
    let msg = DeepSeekMessage {
        role: "assistant".to_string(),
        content: "Hello, world!".to_string(),
        reasoning_content: Some("Let me think...".to_string()),
    };
    // content takes priority
    assert_eq!(DeepSeekProvider::extract_content(&msg), "Hello, world!");
}

#[test]
fn test_extract_content_fallback_to_reasoning() {
    let msg = DeepSeekMessage {
        role: "assistant".to_string(),
        content: "".to_string(),
        reasoning_content: Some("I should help with that.".to_string()),
    };
    // empty content falls back to reasoning_content
    assert_eq!(
        DeepSeekProvider::extract_content(&msg),
        "I should help with that."
    );
}

#[test]
fn test_extract_content_whitespace_trimmed() {
    let msg = DeepSeekMessage {
        role: "assistant".to_string(),
        content: "  Hello, world!  ".to_string(),
        reasoning_content: None,
    };
    assert_eq!(DeepSeekProvider::extract_content(&msg), "Hello, world!");
}

#[test]
fn test_extract_content_both_empty() {
    let msg = DeepSeekMessage {
        role: "assistant".to_string(),
        content: "".to_string(),
        reasoning_content: None,
    };
    assert_eq!(DeepSeekProvider::extract_content(&msg), "");
}

#[test]
fn test_extract_content_whitespace_only_content() {
    let msg = DeepSeekMessage {
        role: "assistant".to_string(),
        content: "  \n\t  ".to_string(),
        reasoning_content: Some("reasoning".to_string()),
    };
    // whitespace-only content should fall back to reasoning_content
    assert_eq!(DeepSeekProvider::extract_content(&msg), "reasoning");
}

// -------------------------------------------------------------------------
// name / provider_display_name / models unit tests
// -------------------------------------------------------------------------

#[test]
fn test_name() {
    let provider = DeepSeekProvider::new("fake-key".into());
    assert_eq!(provider.name(), "deepseek");
}

#[test]
fn test_provider_display_name() {
    let provider = DeepSeekProvider::new("fake-key".into());
    assert_eq!(provider.provider_display_name(), "DeepSeek");
}

#[test]
fn test_models() {
    let provider = DeepSeekProvider::new("fake-key".into());
    let models = provider.models();
    assert!(!models.is_empty());
    assert!(models.contains(&"deepseek-v4-flash"));
}
