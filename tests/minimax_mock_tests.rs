//! MiniMax Provider send() integration tests.
//!
//! Tests the Provider trait interface for MiniMax through the public API,
//! using mockito to mock HTTP responses.

use closeclaw::llm::minimax::MiniMaxProvider;
use closeclaw::llm::provider::{Provider, ProviderError};
use closeclaw::llm::types::{InternalMessage, InternalRequest, RawContentBlock};
use mockito::Server;

fn provider_url(server: &Server) -> String {
    server.url()
}

fn internal_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Say hi".to_string(),
        }],
        temperature: 0.0,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: closeclaw::session::persistence::ReasoningLevel::default(),
        turn_count: None,
    }
}

fn chat_body(model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Say hi"}],
        "temperature": 0.0
    })
}

// --- send() success cases ---

#[tokio::test]
async fn test_provider_send_success_mock() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(
            r#"{
            "choices":[{"message":{"role":"assistant","content":"Hello!"}}],
            "usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}
        }"#,
        )
        .create_async()
        .await;

    let provider = MiniMaxProvider::with_base_url("key".into(), provider_url(&server));
    let resp = provider
        .send(internal_request("Abab5.5-chat"), chat_body("Abab5.5-chat"))
        .await
        .unwrap();

    m.assert_async().await;
    assert!(!resp.content_blocks.is_empty());
    match &resp.content_blocks[0] {
        RawContentBlock::Text(s) => assert!(s.contains("Hello!")),
        other => panic!("Expected Text, got: {:?}", other),
    }
    assert_eq!(resp.usage.prompt_tokens, 5);
    assert_eq!(resp.usage.completion_tokens, 3);
    assert_eq!(resp.usage.total_tokens, Some(8));
}

#[tokio::test]
async fn test_provider_send_reasoning_content_mock() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(
            r#"{
            "choices":[{"message":{"role":"assistant",
            "content":"","reasoning_content":"thinking..."}}],
            "usage":{"prompt_tokens":5,"completion_tokens":10,"total_tokens":15}
        }"#,
        )
        .create_async()
        .await;

    let provider = MiniMaxProvider::with_base_url("key".into(), provider_url(&server));
    let resp = provider
        .send(internal_request("Abab5.5-chat"), chat_body("Abab5.5-chat"))
        .await
        .unwrap();

    m.assert_async().await;
    assert!(resp
        .content_blocks
        .iter()
        .any(|b| { matches!(b, RawContentBlock::Thinking(s) if s.contains("thinking")) }));
}

// --- send() error cases ---

#[tokio::test]
async fn test_provider_send_auth_failure_mock() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = MiniMaxProvider::with_base_url("key".into(), provider_url(&server));
    let err = provider
        .send(internal_request("Abab5.5-chat"), chat_body("Abab5.5-chat"))
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_rate_limit_mock() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(429)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"error":"rate limit exceeded"}"#)
        .create_async()
        .await;

    let provider = MiniMaxProvider::with_base_url("key".into(), provider_url(&server));
    let err = provider
        .send(internal_request("Abab5.5-chat"), chat_body("Abab5.5-chat"))
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_business_error_mock() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"token invalid"}}"#)
        .create_async()
        .await;

    let provider = MiniMaxProvider::with_base_url("key".into(), provider_url(&server));
    let err = provider
        .send(internal_request("Abab5.5-chat"), chat_body("Abab5.5-chat"))
        .await
        .unwrap_err();

    m.assert_async().await;
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("1004"), "should contain 1004");
        }
        other => panic!("Expected Legacy, got: {:?}", other),
    }
}

// --- Provider trait accessor tests ---

#[test]
fn test_provider_id() {
    let provider = MiniMaxProvider::new("key".into());
    assert_eq!(Provider::id(&provider), "minimax");
}

#[test]
fn test_provider_supported_protocols() {
    let provider = MiniMaxProvider::new("key".into());
    let protocols = Provider::supported_protocols(&provider);
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0].as_str(), "anthropic");
}
