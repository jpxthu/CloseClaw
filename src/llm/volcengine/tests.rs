//! Unit tests for the VolcEngine provider.

use super::*;
use mockito::Server;

// -------------------------------------------------------------------------
// Helper utilities
// -------------------------------------------------------------------------

fn provider_url(server: &mockito::Server) -> String {
    server.url()
}

fn chat_request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![crate::llm::Message {
            role: "user".to_string(),
            content: "Say hi".to_string(),
        }],
        temperature: 0.0,
        max_tokens: None,
    }
}

// -------------------------------------------------------------------------
// Chat integration tests via mockito
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/volcengine/chat-success.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "model": "doubao-1.5-pro",
            "messages": [{"role": "user", "content": "Say hi"}],
            "temperature": 0.0
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider.chat(chat_request("doubao-1.5-pro")).await.unwrap();

    m.assert_async().await;
    assert!(!resp.content.is_empty(), "content should be non-empty");
    assert_eq!(
        resp.usage.prompt_tokens > 0,
        true,
        "prompt_tokens should be > 0"
    );
}

#[tokio::test]
async fn test_chat_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/volcengine/error-auth.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("doubao-1.5-pro"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::AuthFailed(_));
}

#[tokio::test]
async fn test_chat_model_not_found_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/volcengine/error-not-found.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("unknown-model"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::ModelNotFound(_));
}

#[tokio::test]
async fn test_chat_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(429)
        .with_header("content-type", "application/json")
        .with_body("{\"error\":{\"code\":\"2001\",\"message\":\"rate limit exceeded\"}}")
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("doubao-1.5-pro"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::RateLimitExceeded);
}

#[tokio::test]
async fn test_chat_business_error_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/volcengine/error-business.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("doubao-1.5-pro"))
        .await
        .unwrap_err();

    m.assert_async().await;
    // Business error with code="1103" should map to ModelNotFound
    matches!(err, LLMError::ModelNotFound(_));
}

// -------------------------------------------------------------------------
// fetch_model_list tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_fetch_model_list_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/volcengine/models-list.json");

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

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), server.url());
    let models = provider.fetch_model_list("fake-key").await.unwrap();

    m.assert_async().await;
    assert!(!models.is_empty(), "expected at least one model");
    for model in &models {
        assert!(
            !model.name.to_lowercase().contains("shutdown"),
            "Shutdown model {} should be filtered",
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
        .with_body(r#"{"error":{"code":"1001","message":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), server.url());
    let err = provider.fetch_model_list("fake-key").await.unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::AuthFailed(_));
}

// -------------------------------------------------------------------------
// Unit tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_extract_content() {
    let msg = VolcEngineMessage {
        role: "assistant".to_string(),
        content: "  Hello, world!  ".to_string(),
    };
    assert_eq!(VolcEngineProvider::extract_content(&msg), "Hello, world!");
}

#[tokio::test]
async fn test_provider_display_name() {
    let provider = VolcEngineProvider::new("fake-key".into());
    assert_eq!(provider.provider_display_name(), "VolcEngine");
}

#[tokio::test]
async fn test_name() {
    let provider = VolcEngineProvider::new("fake-key".into());
    assert_eq!(provider.name(), "volcengine");
}

#[tokio::test]
async fn test_models() {
    let provider = VolcEngineProvider::new("fake-key".into());
    let models = provider.models();
    assert!(!models.is_empty());
    assert!(models.contains(&"doubao-1.5-pro"));
}
