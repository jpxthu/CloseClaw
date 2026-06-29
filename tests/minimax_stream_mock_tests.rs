//! MiniMax Provider send_streaming() integration tests.
//!
//! Tests the streaming SSE interface for MiniMax through the public API,
//! using mockito to mock HTTP streaming responses.

use closeclaw_llm::minimax::MiniMaxProvider;
use closeclaw_llm::provider::{Provider, ProviderError};
use closeclaw_llm::types::{InternalMessage, InternalRequest};
use mockito::Server;

fn provider_url(server: &Server) -> String {
    server.url()
}

fn streaming_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Say hi".to_string(),
            tool_call_id: None,
        }],
        temperature: 0.0,
        max_tokens: None,
        stream: true,
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

fn streaming_body(model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Say hi"}],
        "temperature": 0.0,
        "stream": true
    })
}

// --- send_streaming() success ---

#[tokio::test]
async fn test_provider_send_streaming_success_mock() {
    let mut server = Server::new_async().await;
    let sse_body = "\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
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

    let provider = MiniMaxProvider::with_base_url("key".into(), provider_url(&server));
    let result = provider
        .send_streaming(
            streaming_request("Abab5.5-chat"),
            streaming_body("Abab5.5-chat"),
        )
        .await;

    m.assert_async().await;
    assert!(result.is_ok());

    let mut rx = result.unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    assert_eq!(
        chunks.len(),
        5,
        "should have 5 data chunks (start, 2 deltas, stop, message_stop)"
    );
    assert!(chunks[1].data.contains("text_delta"));
    assert!(chunks[1].data.contains("Hello"));
    assert!(chunks[2].data.contains("text_delta"));
    assert!(chunks[2].data.contains("world"));
}

#[tokio::test]
async fn test_provider_send_streaming_reasoning_mock() {
    let mut server = Server::new_async().await;
    let sse_body = "\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"thinking...\"}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
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

    let provider = MiniMaxProvider::with_base_url("key".into(), provider_url(&server));
    let result = provider
        .send_streaming(
            streaming_request("Abab5.5-chat"),
            streaming_body("Abab5.5-chat"),
        )
        .await;

    m.assert_async().await;
    assert!(result.is_ok());

    let mut rx = result.unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    assert_eq!(
        chunks.len(),
        4,
        "should have 4 data chunks (start, thinking delta, stop, message_stop)"
    );
    assert!(chunks[1].data.contains("thinking_delta"));
    assert!(chunks[1].data.contains("thinking..."));
}

// --- send_streaming() error cases ---

#[tokio::test]
async fn test_provider_send_streaming_auth_failure_mock() {
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
        .send_streaming(
            streaming_request("Abab5.5-chat"),
            streaming_body("Abab5.5-chat"),
        )
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_streaming_rate_limit_mock() {
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
        .send_streaming(
            streaming_request("Abab5.5-chat"),
            streaming_body("Abab5.5-chat"),
        )
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}
