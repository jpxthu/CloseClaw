//! Unit tests for the VolcEngine Provider implementation.

use super::*;
use crate::llm::provider::Provider;
use crate::llm::types::{InternalMessage, InternalRequest, RawContentBlock};
use crate::session::persistence::ReasoningLevel;
use serde_json::json;

// -------------------------------------------------------------------------
// Helper utilities
// -------------------------------------------------------------------------

fn provider_url(server: &mockito::Server) -> String {
    server.url()
}

fn make_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".into(),
            content: "Say hi".into(),
        }],
        temperature: 0.0,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
    }
}

// -------------------------------------------------------------------------
// Provider accessor tests
// -------------------------------------------------------------------------

#[test]
fn test_provider_accessors() {
    let provider = VolcEngineProvider::new("fake-key".into());
    assert_eq!(provider.id(), "volcengine");
    assert_eq!(provider.base_url(), VOLCENGINE_API_URL);
    assert_eq!(provider.api_key(), "fake-key");
    let protocols = provider.supported_protocols();
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0].as_str(), "openai");
    let _ = provider.http_client();
    assert!(provider.default_headers().is_empty());

    // Test custom base URL with a separate instance
    let custom =
        VolcEngineProvider::with_base_url("sk-test".into(), "https://custom.api.com".into());
    assert_eq!(custom.base_url(), "https://custom.api.com");
}

// -------------------------------------------------------------------------
// send() success tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_send_success() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../../tests/fixtures/llm/volcengine/chat-success.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer fake-key".into()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let req = make_request("doubao-1.5-pro");
    let body = json!({
        "model": "doubao-1.5-pro",
        "messages": [{"role": "user", "content": "Say hi"}],
        "temperature": 0.0
    });

    let resp = provider.send(req, body).await.unwrap();

    m.assert_async().await;
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        RawContentBlock::Text("Hello! I'm doing well, thank you for asking.".into())
    );
    assert_eq!(resp.usage.prompt_tokens, 12);
    assert_eq!(resp.usage.completion_tokens, 9);
    assert_eq!(resp.usage.total_tokens, Some(21));
    assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
}

// -------------------------------------------------------------------------
// send() error tests (HTTP status code mapping)
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_send_auth_failure() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../../tests/fixtures/llm/volcengine/error-auth.json");
    let m = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let req = make_request("doubao-1.5-pro");
    let body = json!({"model": "doubao-1.5-pro", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::llm::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("401"),
                "expected 401 in error message, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error for 401, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_send_model_not_found() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../../tests/fixtures/llm/volcengine/error-not-found.json");
    let m = server
        .mock("POST", "/chat/completions")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let req = make_request("unknown-model");
    let body = json!({"model": "unknown-model", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::llm::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("404"),
                "expected 404 in error message, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error for 404, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_send_rate_limit() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/chat/completions")
        .with_status(429)
        .with_header("content-type", "application/json")
        .with_body("{\"error\":{\"code\":\"2001\",\"message\":\"rate limit exceeded\"}}")
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let req = make_request("doubao-1.5-pro");
    let body = json!({"model": "doubao-1.5-pro", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::llm::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("429"),
                "expected 429 in error message, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error for 429, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_send_business_error() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../../tests/fixtures/llm/volcengine/error-business.json");
    let m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let req = make_request("doubao-1.5-pro");
    let body = json!({"model": "doubao-1.5-pro", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    // Business error with code="1103" should produce a Legacy error
    match err {
        crate::llm::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("1103"),
                "expected business error code 1103, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error for business error, got: {:?}", other),
    }
}

// -------------------------------------------------------------------------
// send_streaming() tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_send_streaming_success() {
    let mut server = mockito::Server::new_async().await;

    // Build SSE response body with multiple chunks and [DONE]
    let sse_body = "\
data: {\"id\":\"volc-sse-001\",\"object\":\"chat.completion.chunk\",\"model\":\"doubao-1.5-pro\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}

data: {\"id\":\"volc-sse-001\",\"object\":\"chat.completion.chunk\",\"model\":\"doubao-1.5-pro\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}

data: {\"id\":\"volc-sse-001\",\"object\":\"chat.completion.chunk\",\"model\":\"doubao-1.5-pro\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}

data: [DONE]

";

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer fake-key".into()),
        )
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(sse_body)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let mut req = make_request("doubao-1.5-pro");
    req.stream = true;
    let body = json!({
        "model": "doubao-1.5-pro",
        "messages": [{"role": "user", "content": "Say hi"}],
        "stream": true
    });

    let mut rx = provider
        .send_streaming(req, body)
        .await
        .expect("send_streaming should succeed");

    m.assert_async().await;

    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }

    // Should receive 3 chunks (the [DONE] frame does not produce a chunk)
    assert_eq!(
        chunks.len(),
        3,
        "expected 3 SSE chunks, got {}",
        chunks.len()
    );

    // Each chunk should be a RawSseChunk with event_type "message"
    assert!(chunks[0].data.contains("Hello"));
    assert_eq!(chunks[0].event_type, "message");

    assert!(chunks[1].data.contains(" world"));
    assert_eq!(chunks[1].event_type, "message");

    assert!(chunks[2].data.contains("finish_reason"));
    assert_eq!(chunks[2].event_type, "message");
}

#[tokio::test]
async fn test_send_streaming_error_401() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"invalid_api_key","message":"Invalid"}}"#)
        .create_async()
        .await;

    let provider = VolcEngineProvider::with_base_url("fake-key".into(), provider_url(&server));
    let mut req = make_request("doubao-1.5-pro");
    req.stream = true;
    let body = json!({"model": "doubao-1.5-pro", "messages": [], "stream": true});

    let err = provider.send_streaming(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::llm::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("401"),
                "expected 401 in error message, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error for 401, got: {:?}", other),
    }
}

// -------------------------------------------------------------------------
// fetch_model_list tests (ModelLister interface unchanged)
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_fetch_model_list_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../../tests/fixtures/llm/volcengine/models-list.json");

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
