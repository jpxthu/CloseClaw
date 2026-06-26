//! Unit tests for the MiniMax provider.

use super::*;
use crate::llm::{ModelLister, Provider};

// TODO: Rewrite with v2 fixture (minimax/MiniMax-M3/anthropic/)
// #[test]
// fn test_simple_chat_deserialize_and_extract() { ... }
// #[test]
// fn test_simple_chat_content_priority_over_reasoning() { ... }

// --- Provider trait tests ---

#[test]
fn test_provider_id() {
    let provider = MiniMaxProvider::new("key".into());
    assert_eq!(Provider::id(&provider), "minimax");
}

#[test]
fn test_provider_base_url() {
    let provider = MiniMaxProvider::new("key".into());
    assert_eq!(
        Provider::base_url(&provider),
        "https://api.minimax.chat/v1/chat/completions"
    );
}

#[test]
fn test_provider_api_key() {
    let provider = MiniMaxProvider::new("my-key".into());
    assert_eq!(Provider::api_key(&provider), "my-key");
}

#[test]
fn test_provider_supported_protocols() {
    let provider = MiniMaxProvider::new("key".into());
    let protocols = Provider::supported_protocols(&provider);
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0].as_str(), "anthropic");
}

#[test]
fn test_provider_http_client() {
    let provider = MiniMaxProvider::new("key".into());
    // Just verify it returns a valid reference
    let _ = Provider::http_client(&provider);
}

#[test]
fn test_provider_default_headers() {
    let provider = MiniMaxProvider::new("key".into());
    let headers = Provider::default_headers(&provider);
    assert!(headers.is_empty());
}

// --- Provider send() via mockito ---

fn mock_provider(server: &mockito::Server) -> MiniMaxProvider {
    MiniMaxProvider::with_http_client("test-key".into(), server.url(), reqwest::Client::new())
}

fn create_internal_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![crate::llm::types::InternalMessage {
            role: "user".into(),
            content: "hi".into(),
        }],
        temperature: 0.7,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: Default::default(),
        turn_count: None,
    }
}

#[tokio::test]
async fn test_provider_send_success_mock() {
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
            "usage":{"completion_tokens":10,"prompt_tokens":5,"total_tokens":15}
        }"#,
        )
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({
        "model": "Abab5.5-chat",
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.7,
        "stream": false
    });
    let result = Provider::send(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert!(!resp.content_blocks.is_empty());
    assert_eq!(resp.usage.prompt_tokens, 5);
    assert_eq!(resp.usage.completion_tokens, 10);
    assert_eq!(resp.usage.total_tokens, Some(15));
}

#[tokio::test]
async fn test_provider_send_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send(&provider, req, body).await.unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(429)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"error":"rate limit exceeded"}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send(&provider, req, body).await.unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_business_error_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"token invalid"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send(&provider, req, body).await.unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(ref msg) if msg.contains("1004")));
}

#[tokio::test]
async fn test_provider_send_reasoning_content_mock() {
    let mut server = mockito::Server::new_async().await;
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

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let result = Provider::send(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    // Should have Thinking block from reasoning_content
    assert!(resp
        .content_blocks
        .iter()
        .any(|b| matches!(b, RawContentBlock::Thinking { .. })));
}

// --- Provider send_streaming() via mockito ---

fn create_streaming_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![crate::llm::types::InternalMessage {
            role: "user".into(),
            content: "hi".into(),
        }],
        temperature: 0.7,
        max_tokens: None,
        stream: true,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        tools: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: Default::default(),
        turn_count: None,
    }
}

#[tokio::test]
async fn test_provider_send_streaming_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let sse_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n",
        "\n",
        "data: [DONE]\n",
        "\n",
    );
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "text/event-stream")
        .with_chunked_body(move |w| {
            w.write_all(sse_body.as_bytes()).unwrap();
            Ok(())
        })
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({
        "model": "Abab5.5-chat",
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.7,
        "stream": true
    });
    let result = Provider::send_streaming(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let mut rx = result.unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    assert_eq!(
        chunks.len(),
        2,
        "should have 2 data chunks (excluding [DONE])"
    );
    assert!(chunks[0].data.contains("Hello"));
    assert!(chunks[1].data.contains("world"));
}

#[tokio::test]
async fn test_provider_send_streaming_reasoning_mock() {
    let mut server = mockito::Server::new_async().await;
    let sse_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking...\"}}]}\n",
        "\n",
        "data: [DONE]\n",
        "\n",
    );
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "text/event-stream")
        .with_chunked_body(move |w| {
            w.write_all(sse_body.as_bytes()).unwrap();
            Ok(())
        })
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({
        "model": "Abab5.5-chat",
        "stream": true
    });
    let result = Provider::send_streaming(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let mut rx = result.unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].data.contains("reasoning_content"));
}

#[tokio::test]
async fn test_provider_send_streaming_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send_streaming(&provider, req, body)
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_streaming_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(429)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"error":"rate limit exceeded"}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send_streaming(&provider, req, body)
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

// --- fetch_model_list knowledge base filling tests ---

#[tokio::test]
async fn test_fetch_model_list_uses_knowledge_base() {
    let mut server = mockito::Server::new_async().await;
    let api_response = serde_json::json!({
        "data": [
            {"id": "MiniMax-M2.7", "owned_by": "minimax"},
            {"id": "MiniMax-M2", "owned_by": "minimax"}
        ]
    });
    let m = server
        .mock("GET", "/v1/models")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(api_response.to_string())
        .create_async()
        .await;

    let provider =
        MiniMaxProvider::with_http_client("test-key".into(), server.url(), reqwest::Client::new());
    let models = ModelLister::fetch_model_list(&provider, "test-key")
        .await
        .unwrap();

    m.assert_async().await;
    assert_eq!(models.len(), 2);

    // MiniMax-M2.7: knowledge base has reasoning=true, context_window=204800
    let m27 = models.iter().find(|m| m.id == "MiniMax-M2.7").unwrap();
    assert!(
        m27.reasoning,
        "knowledge base should set reasoning=true for M2.7"
    );
    assert_eq!(m27.context_window, 204_800);

    // MiniMax-M2: knowledge base has reasoning=true, context_window=204800
    let m2 = models.iter().find(|m| m.id == "MiniMax-M2").unwrap();
    assert!(
        m2.reasoning,
        "knowledge base should set reasoning=true for M2"
    );
    assert_eq!(m2.context_window, 204_800);
}

#[tokio::test]
async fn test_fetch_model_list_unknown_model_uses_fallback() {
    let mut server = mockito::Server::new_async().await;
    let api_response = serde_json::json!({
        "data": [
            {"id": "unknown-future-model", "owned_by": "minimax"}
        ]
    });
    let m = server
        .mock("GET", "/v1/models")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(api_response.to_string())
        .create_async()
        .await;

    let provider =
        MiniMaxProvider::with_http_client("test-key".into(), server.url(), reqwest::Client::new());
    let models = ModelLister::fetch_model_list(&provider, "test-key")
        .await
        .unwrap();

    m.assert_async().await;
    assert_eq!(models.len(), 1);
    // Unknown model: fallback defaults (context_window=32768, reasoning=false)
    let model = &models[0];
    assert_eq!(model.id, "unknown-future-model");
    assert_eq!(model.context_window, 32_768);
    assert!(!model.reasoning);
}
