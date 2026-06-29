//! Unit tests for the MiMo Provider implementation.

use super::*;
use crate::provider::Provider;
use crate::types::{InternalMessage, InternalRequest};
use serde_json::json;

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn provider_url(server: &mockito::Server) -> String {
    server.url()
}

fn make_sse_body(data_lines: &[&str]) -> String {
    let events: Vec<String> = data_lines.iter().map(|d| format!("data: {}", d)).collect();
    events.join("\n\n") + "\n\n"
}

async fn collect_chunks(rx: &mut mpsc::Receiver<RawSseChunk>) -> Vec<RawSseChunk> {
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    chunks
}

fn make_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".into(),
            content: "Hello".into(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: Some(100),
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: closeclaw_session::persistence::ReasoningLevel::default(),
        turn_count: None,
    }
}

// ---------------------------------------------------------------------------
// Construction tests
// ---------------------------------------------------------------------------

#[test]
fn test_new_sets_default_base_url_and_api_key() {
    let provider = MimoProvider::new("sk-test-key".into());
    assert_eq!(provider.base_url(), MIMO_BASE_URL);
    assert_eq!(provider.api_key(), "sk-test-key");
}

#[test]
fn test_from_env_returns_none_when_var_unset() {
    // In CI/test env MIMO_API_KEY is not set, so from_env() should return None.
    // We cannot modify env vars (forbidden by CONTRIBUTING.md), so we rely on the env being unset.
    let result = MimoProvider::from_env();
    // If by chance the var is set in the environment, skip this assertion.
    if std::env::var("MIMO_API_KEY").is_err() {
        assert!(
            result.is_none(),
            "from_env should return None when MIMO_API_KEY is unset"
        );
    }
}

#[test]
fn test_with_base_url_sets_custom_url() {
    let provider = MimoProvider::with_base_url("sk-custom".into(), "https://custom.example.com/v1");
    assert_eq!(provider.base_url(), "https://custom.example.com/v1");
    assert_eq!(provider.api_key(), "sk-custom");
}

// ---------------------------------------------------------------------------
// Provider trait tests
// ---------------------------------------------------------------------------

#[test]
fn test_provider_id_returns_mimo() {
    let provider = MimoProvider::new("sk-test".into());
    assert_eq!(provider.id(), "mimo");
}

#[test]
fn test_provider_supported_protocols_returns_openai() {
    let provider = MimoProvider::new("sk-test".into());
    let protocols = provider.supported_protocols();
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0].as_str(), "openai");
}

#[test]
fn test_provider_http_client_has_no_custom_headers() {
    let provider = MimoProvider::new("sk-test".into());
    // MimoProvider uses no custom default headers; Bearer auth is added per-request.
    assert!(
        provider.default_headers().is_empty(),
        "MimoProvider should have no default headers"
    );
}

// ---------------------------------------------------------------------------
// chat_url construction tests
// ---------------------------------------------------------------------------

#[test]
fn test_chat_url_default_base_url() {
    let provider = MimoProvider::new("sk-test".into());
    assert_eq!(
        provider.chat_url(),
        "https://api.xiaomimimo.com/v1/chat/completions"
    );
}

#[test]
fn test_chat_url_custom_base_url() {
    let provider = MimoProvider::with_base_url("sk-test".into(), "https://custom.example.com/v1");
    assert_eq!(
        provider.chat_url(),
        "https://custom.example.com/v1/chat/completions"
    );
}

// ---------------------------------------------------------------------------
// send() success tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_success() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "mimo-001",
        "model": "mimo-7b",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Hello from MiMo!"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": 10,
            "total_tokens": 15
        }
    });

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({
        "model": "mimo-7b",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let resp = provider.send(req, body).await.expect("send should succeed");

    m.assert_async().await;
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Text("Hello from MiMo!".into())
    );
    assert_eq!(resp.usage.prompt_tokens, 5);
    assert_eq!(resp.usage.completion_tokens, 10);
    assert_eq!(resp.usage.total_tokens, Some(15));
    assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn test_send_success_with_reasoning_content() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "mimo-reason-001",
        "model": "mimo-7b",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "The answer is 42.",
                "reasoning_content": "Let me think step by step..."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 15,
            "total_tokens": 25
        }
    });

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({
        "model": "mimo-7b",
        "messages": [{"role": "user", "content": "What is the answer?"}]
    });

    let resp = provider.send(req, body).await.expect("send should succeed");

    m.assert_async().await;
    // MimoProvider currently only parses content, not reasoning_content
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Text("The answer is 42.".into())
    );
}

#[tokio::test]
async fn test_send_no_choices_returns_error() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "mimo-no-choices",
        "model": "mimo-7b",
        "choices": [],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
        }
    });

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({"model": "mimo-7b", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("no choices"),
                "expected 'no choices' error, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// send() error tests (HTTP status code mapping)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_error_401() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"invalid_api_key","message":"Invalid API key"}}"#)
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({"model": "mimo-7b", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(msg.contains("401"), "expected 401 in error, got: {}", msg);
        }
        other => panic!("expected Legacy error for 401, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_send_error_429() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(429)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"rate_limit_exceeded","message":"rate limit exceeded"}}"#)
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({"model": "mimo-7b", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(msg.contains("429"), "expected 429 in error, got: {}", msg);
        }
        other => panic!("expected Legacy error for 429, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_send_error_500() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(500)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"internal_error","message":"Internal server error"}}"#)
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({"model": "mimo-7b", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(msg.contains("500"), "expected 500 in error, got: {}", msg);
        }
        other => panic!("expected Legacy error for 500, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// send_streaming() tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_streaming_success() {
    let mut server = mockito::Server::new_async().await;

    let d1 = r#"{"id":"m","object":"c","choices":[{"delta":{"content":"Hello"},"fin":null}]}"#;
    let d2 = r#"{"id":"m","object":"c","choices":[{"delta":{"content":" world"},"fin":null}]}"#;
    let d3 = r#"{"id":"m","object":"c","choices":[{"delta":{},"finish_reason":"stop"}]}"#;
    let sse_body = make_sse_body(&[d1, d2, d3, "[DONE]"]);

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer sk-.*".into()),
        )
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(sse_body)
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let mut req = make_request("mimo-7b");
    req.stream = true;
    let body = json!({
        "model": "mimo-7b",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true,
    });

    let mut rx = provider
        .send_streaming(req, body)
        .await
        .expect("send_streaming should succeed");

    m.assert_async().await;
    let chunks = collect_chunks(&mut rx).await;

    assert_eq!(chunks.len(), 3);
    assert!(chunks[0].data.contains("Hello"));
    assert!(chunks[1].data.contains(" world"));
    assert!(chunks[2].data.contains("finish_reason"));
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

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let mut req = make_request("mimo-7b");
    req.stream = true;
    let body = json!({"model": "mimo-7b", "messages": [], "stream": true});

    let err = provider.send_streaming(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(msg.contains("401"));
        }
        other => panic!("expected Legacy error for 401, got: {:?}", other),
    }
}
