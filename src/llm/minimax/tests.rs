//! Unit tests for the MiniMax provider.

use super::*;
use crate::llm::Provider;

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
        session_id: None,
        reasoning_level: Default::default(),
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
        .any(|b| matches!(b, RawContentBlock::Thinking(_))));
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
        system_blocks: None,
        session_id: None,
        reasoning_level: Default::default(),
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

// Legacy LLMProvider/ModelLister tests removed in Step 1.3
