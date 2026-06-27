//! Unit tests for the DeepSeek Provider implementation.

use super::*;
use crate::provider::Provider;
use crate::types::{InternalMessage, InternalRequest, RawContentBlock};
use serde_json::json;

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn provider_url(server: &mockito::Server) -> String {
    server.url()
}

fn make_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".into(),
            content: "Hello".into(),
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
// Provider accessor tests
// ---------------------------------------------------------------------------

#[test]
fn test_provider_accessors() {
    let provider = DeepSeekProvider::new("sk-secret-key".into());
    assert_eq!(provider.id(), "deepseek");
    assert_eq!(provider.base_url(), DEEPSEEK_API_URL);
    assert_eq!(provider.api_key(), "sk-secret-key");
    let protocols = provider.supported_protocols();
    assert_eq!(protocols.len(), 2);
    assert_eq!(protocols[0].as_str(), "openai");
    assert_eq!(protocols[1].as_str(), "anthropic");
    let _ = provider.http_client();
    assert!(provider.default_headers().is_empty());

    // Test custom base URL with a separate instance
    let custom = DeepSeekProvider::with_base_url("sk-test".into(), "https://custom.api.com".into());
    assert_eq!(custom.base_url(), "https://custom.api.com");
}

// ---------------------------------------------------------------------------
// send() success tests
// ---------------------------------------------------------------------------

// TODO: Rewrite with v2 fixture (deepseek/deepseek-v4-flash/openai/simple.json)
// #[tokio::test]
// async fn test_send_success_text_only() { ... }

#[tokio::test]
async fn test_send_success_with_reasoning_content() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "ds-reason-001",
        "model": "deepseek-v4-pro",
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

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-pro");
    let body = json!({
        "model": "deepseek-v4-pro",
        "messages": [{"role": "user", "content": "What is the answer?"}]
    });

    let resp = provider.send(req, body).await.expect("send should succeed");

    m.assert_async().await;
    // Should have Thinking block first, then Text block
    assert_eq!(resp.content_blocks.len(), 2);
    assert_eq!(
        resp.content_blocks[0],
        RawContentBlock::Thinking {
            thinking: "Let me think step by step...".into(),
            signature: None
        }
    );
    assert_eq!(
        resp.content_blocks[1],
        RawContentBlock::Text("The answer is 42.".into())
    );
}

#[tokio::test]
async fn test_send_success_empty_content_fallback() {
    let mut server = mockito::Server::new_async().await;

    // Response with empty content and no reasoning_content
    let response_body = json!({
        "id": "ds-empty-001",
        "model": "deepseek-v4-flash",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": ""
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": 0,
            "total_tokens": 5
        }
    });

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-flash");
    let body = json!({"model": "deepseek-v4-flash", "messages": []});

    let resp = provider.send(req, body).await.expect("send should succeed");

    m.assert_async().await;
    // Empty content should produce a single Text block with empty string
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(resp.content_blocks[0], RawContentBlock::Text(String::new()));
}

#[tokio::test]
async fn test_send_success_no_choices_error() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "ds-no-choices-001",
        "model": "deepseek-v4-flash",
        "choices": [],
        "usage": null
    });

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-flash");
    let body = json!({"model": "deepseek-v4-flash", "messages": []});

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
async fn test_send_error_401_auth() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"invalid_api_key","message":"Invalid API key"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-flash");
    let body = json!({"model": "deepseek-v4-flash", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
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
async fn test_send_error_429_rate_limit() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(429)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"rate_limit_exceeded","message":"rate limit exceeded"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-flash");
    let body = json!({"model": "deepseek-v4-flash", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
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
async fn test_send_error_404_not_found() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"model_not_found","message":"Model not found"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("unknown-model");
    let body = json!({"model": "unknown-model", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
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
async fn test_send_business_error_in_body() {
    let mut server = mockito::Server::new_async().await;

    // HTTP 200 but with error in body
    let response_body = json!({
        "id": "ds-biz-err-001",
        "model": "deepseek-v4-flash",
        "choices": [{
            "message": {"role": "assistant", "content": ""},
            "finish_reason": null
        }],
        "error": {
            "code": "context_length_exceeded",
            "message": "Maximum context length exceeded"
        }
    });

    let m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-flash");
    let body = json!({"model": "deepseek-v4-flash", "messages": []});

    let err = provider.send(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("context_length_exceeded"),
                "expected business error, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error for business error, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// send_streaming() tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_streaming_success() {
    let mut server = mockito::Server::new_async().await;

    // Build SSE response body with multiple chunks and [DONE]
    let sse_body = "\
data: {\"id\":\"ds-sse-001\",\"object\":\"chat.completion.chunk\",\"model\":\"deepseek-v4-flash\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}

data: {\"id\":\"ds-sse-001\",\"object\":\"chat.completion.chunk\",\"model\":\"deepseek-v4-flash\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}

data: {\"id\":\"ds-sse-001\",\"object\":\"chat.completion.chunk\",\"model\":\"deepseek-v4-flash\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}

data: [DONE]

";

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

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-flash");
    let mut req = req;
    req.stream = true;
    let body = json!({
        "model": "deepseek-v4-flash",
        "messages": [{"role": "user", "content": "Hello"}],
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

    // Each chunk should be a RawSseChunk with the data payload
    assert!(chunks[0].data.contains("Hello"));
    assert!(chunks[0].event_type == "message");

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

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let mut req = make_request("deepseek-v4-flash");
    req.stream = true;
    let body = json!({"model": "deepseek-v4-flash", "messages": [], "stream": true});

    let err = provider.send_streaming(req, body).await.unwrap_err();

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(msg.contains("401"));
        }
        other => panic!("expected Legacy error for 401, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Anthropic protocol path tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_anthropic_protocol_success() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "msg-anthropic-001",
        "type": "message",
        "role": "assistant",
        "content": [
            {
                "type": "thinking",
                "thinking": "Let me reason through this...",
                "signature": "sig-abc123"
            },
            {
                "type": "text",
                "text": "The answer is 42."
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 20
        }
    });

    let m = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-pro");
    let body = json!({
        "model": "deepseek-v4-pro",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": "What is the answer?"}]
        }]
    });

    let resp = provider.send(req, body).await.expect("send should succeed");
    m.assert_async().await;

    assert_eq!(resp.content_blocks.len(), 2);
    assert_eq!(
        resp.content_blocks[0],
        RawContentBlock::Thinking {
            thinking: "Let me reason through this...".into(),
            signature: Some("sig-abc123".into()),
        }
    );
    assert_eq!(
        resp.content_blocks[1],
        RawContentBlock::Text("The answer is 42.".into())
    );
    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 20);
}

#[tokio::test]
async fn test_send_anthropic_thinking_with_signature() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "msg-anthropic-002",
        "type": "message",
        "role": "assistant",
        "content": [
            {
                "type": "thinking",
                "thinking": "Deep reasoning trace.",
                "signature": "ESig-xyz-789"
            },
            {
                "type": "text",
                "text": "Done."
            }
        ],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 5, "output_tokens": 8 }
    });

    let m = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let req = make_request("deepseek-v4-pro");
    let body = json!({
        "model": "deepseek-v4-pro",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        }]
    });

    let resp = provider.send(req, body).await.expect("send should succeed");
    m.assert_async().await;

    // Thinking block should carry the signature
    match &resp.content_blocks[0] {
        RawContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "Deep reasoning trace.");
            assert_eq!(signature.as_deref(), Some("ESig-xyz-789"));
        }
        other => panic!("expected Thinking block, got {:?}", other),
    }
}

#[tokio::test]
async fn test_send_streaming_anthropic_protocol() {
    let mut server = mockito::Server::new_async().await;

    // Anthropic SSE format: events separated by \n\n
    let sse_body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-1\",\"role\":\"assistant\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
        "\n",
    );

    let m = server
        .mock("POST", "/v1/messages")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer sk-.*".into()),
        )
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(sse_body)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let mut req = make_request("deepseek-v4-pro");
    req.stream = true;
    let body = json!({
        "model": "deepseek-v4-pro",
        "max_tokens": 1024,
        "stream": true,
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": "Hi"}]
        }]
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

    // Should receive the message_start + 2 content_block_delta + message_delta + message_stop
    // = 5 data frames (event-only lines without data are not sent as chunks)
    assert!(
        chunks.len() >= 2,
        "expected >= 2 SSE chunks, got {}",
        chunks.len()
    );

    // Verify we got the text delta chunks
    let all_data: String = chunks.iter().map(|c| c.data.as_str()).collect();
    assert!(all_data.contains("Hello"), "expected 'Hello' in chunks");
    assert!(all_data.contains("world"), "expected 'world' in chunks");
}

// ---------------------------------------------------------------------------
// detect_is_anthropic tests
// ---------------------------------------------------------------------------

#[test]
fn test_detect_anthropic_with_array_content() {
    let body = json!({
        "model": "deepseek-v4-pro",
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": "Hello"}]
        }]
    });
    assert!(detect_is_anthropic(&body));
}

#[test]
fn test_detect_openai_with_string_content() {
    let body = json!({
        "model": "deepseek-v4-flash",
        "messages": [{
            "role": "user",
            "content": "Hello"
        }]
    });
    assert!(!detect_is_anthropic(&body));
}

#[test]
fn test_detect_anthropic_with_system_field() {
    // Top-level "system" field is Anthropic-only;
    // even with string content in messages, it should return true.
    let body = json!({
        "model": "deepseek-v4-pro",
        "max_tokens": 1024,
        "system": "You are a helpful assistant.",
        "messages": [{
            "role": "user",
            "content": "Hello"
        }]
    });
    assert!(detect_is_anthropic(&body));
}

#[test]
fn test_detect_anthropic_with_system_and_array_content() {
    // Both primary (array content) and secondary (system field) signals;
    // should return true as a double confirmation.
    let body = json!({
        "model": "deepseek-v4-pro",
        "max_tokens": 1024,
        "system": [{"type": "text", "text": "You are helpful."}],
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": "Hi"}]
        }]
    });
    assert!(detect_is_anthropic(&body));
}

#[test]
fn test_detect_empty_messages() {
    let body = json!({
        "model": "deepseek-v4-flash",
        "messages": []
    });
    assert!(!detect_is_anthropic(&body));
}

// ---------------------------------------------------------------------------
// fetch_balance tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fetch_balance_success() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "is_available": true,
        "balance_infos": [{
            "currency": "USD",
            "total_balance": 42.50,
            "granted_balance": 10.00,
            "topped_up_balance": 32.50
        }]
    });

    let m = server
        .mock("GET", "/user/balance")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let balance = provider
        .fetch_balance(&server.url())
        .await
        .expect("fetch_balance should succeed");

    m.assert_async().await;
    assert!(balance.is_available);
    assert_eq!(balance.balance_infos.len(), 1);
    assert_eq!(balance.balance_infos[0].currency, "USD");
    assert_eq!(balance.balance_infos[0].total_balance, 42.50);
    assert_eq!(balance.balance_infos[0].granted_balance, 10.00);
    assert_eq!(balance.balance_infos[0].topped_up_balance, 32.50);
}

#[tokio::test]
async fn test_fetch_balance_error() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("GET", "/user/balance")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"message":"Unauthorized"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let err = provider
        .fetch_balance(&server.url())
        .await
        .expect_err("fetch_balance should fail on 401");

    m.assert_async().await;
    match err {
        crate::provider::ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("401"),
                "expected 401 in error message, got: {}",
                msg
            );
        }
        other => panic!("expected Legacy error for 401, got: {:?}", other),
    }
}

mod tests_model_lister;
