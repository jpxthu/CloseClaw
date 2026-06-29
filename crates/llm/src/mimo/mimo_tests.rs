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
fn test_provider_supported_protocols_returns_openai_and_anthropic() {
    let provider = MimoProvider::new("sk-test".into());
    let protocols = provider.supported_protocols();
    assert_eq!(protocols.len(), 2);
    assert_eq!(protocols[0].as_str(), "openai");
    assert_eq!(protocols[1].as_str(), "anthropic");
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
// messages_url construction tests
// ---------------------------------------------------------------------------

#[test]
fn test_messages_url_default_base_url() {
    let provider = MimoProvider::new("sk-test".into());
    assert_eq!(
        provider.messages_url(),
        "https://api.xiaomimimo.com/v1/messages"
    );
}

#[test]
fn test_messages_url_custom_base_url() {
    let provider = MimoProvider::with_base_url("sk-test".into(), "https://custom.example.com/v1");
    assert_eq!(
        provider.messages_url(),
        "https://custom.example.com/v1/messages"
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
    // Now correctly parses reasoning_content into a Thinking block (signature: None)
    assert_eq!(resp.content_blocks.len(), 2);
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Thinking {
            thinking: "Let me think step by step...".into(),
            signature: None,
        }
    );
    assert_eq!(
        resp.content_blocks[1],
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

// ---------------------------------------------------------------------------
// Anthropic response parsing tests (via send() with mock)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_anthropic_success_text_only() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "mimo-ant-001",
        "type": "message",
        "role": "assistant",
        "content": [
            { "type": "text", "text": "Hello from Anthropic!" }
        ],
        "model": "mimo-7b",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 20
        }
    });

    let m = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({
        "model": "mimo-7b",
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
        "max_tokens": 100
    });

    let resp = provider.send(req, body).await.expect("send should succeed");

    m.assert_async().await;
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Text("Hello from Anthropic!".into())
    );
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 20);
    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));
}

#[tokio::test]
async fn test_send_anthropic_success_with_thinking() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "mimo-ant-002",
        "type": "message",
        "role": "assistant",
        "content": [
            { "type": "thinking", "thinking": "Analyzing the question..." },
            { "type": "text", "text": "The answer is 42." }
        ],
        "model": "mimo-7b",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 15,
            "output_tokens": 30
        }
    });

    let m = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({
        "model": "mimo-7b",
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "What is the answer?" }] }],
        "max_tokens": 100
    });

    let resp = provider.send(req, body).await.expect("send should succeed");

    m.assert_async().await;
    assert_eq!(resp.content_blocks.len(), 2);
    // Thinking block: signature must be Some(String::new()) per MiMo docs
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Thinking {
            thinking: "Analyzing the question...".into(),
            signature: Some(String::new()),
        }
    );
    assert_eq!(
        resp.content_blocks[1],
        crate::types::RawContentBlock::Text("The answer is 42.".into())
    );
}

// ---------------------------------------------------------------------------
// Anthropic response parsing (direct unit tests)
// ---------------------------------------------------------------------------

#[test]
fn test_parse_anthropic_response_text_only() {
    let body = json!({
        "id": "msg-001",
        "content": [
            { "type": "text", "text": "Hello!" }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 5
        }
    });

    let resp = parse_anthropic_response(body).expect("parse should succeed");
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Text("Hello!".into())
    );
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));
}

#[test]
fn test_parse_anthropic_response_thinking_signature_always_empty() {
    let body = json!({
        "content": [
            { "type": "thinking", "thinking": "Let me think..." },
            { "type": "text", "text": "Result" }
        ],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 1, "output_tokens": 1 }
    });

    let resp = parse_anthropic_response(body).expect("parse should succeed");
    assert_eq!(resp.content_blocks.len(), 2);
    // Doc requirement: signature is always an empty string for Anthropic protocol
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Thinking {
            thinking: "Let me think...".into(),
            signature: Some(String::new()),
        }
    );
}

#[test]
fn test_parse_anthropic_usage_with_cache_read_tokens() {
    let body = json!({
        "content": [{ "type": "text", "text": "Hi" }],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "total_tokens": 150,
            "cache_read_input_tokens": 30
        }
    });

    let resp = parse_anthropic_response(body).expect("parse should succeed");
    assert_eq!(resp.usage.prompt_tokens, 100);
    assert_eq!(resp.usage.completion_tokens, 50);
    assert_eq!(resp.usage.total_tokens, Some(150));
    assert_eq!(resp.usage.cache_read_tokens, Some(30));
    assert_eq!(resp.usage.cache_write_tokens, None);
}

#[test]
fn test_parse_anthropic_usage_without_optional_fields() {
    let body = json!({
        "content": [{ "type": "text", "text": "Hi" }],
        "usage": {
            "input_tokens": 10,
            "output_tokens": 5
        }
    });

    let resp = parse_anthropic_response(body).expect("parse should succeed");
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.usage.total_tokens, None);
    assert_eq!(resp.usage.cache_read_tokens, None);
}

#[test]
fn test_parse_content_block_unknown_type_returns_none() {
    let item = json!({ "type": "image", "source": "..." });
    assert!(parse_content_block(&item).is_none());
}

// ---------------------------------------------------------------------------
// Protocol detection tests
// ---------------------------------------------------------------------------

#[test]
fn test_detect_is_anthropic_openai_format_returns_false() {
    // OpenAI: plain string content, no system field
    let body = json!({
        "model": "mimo-7b",
        "messages": [{ "role": "user", "content": "Hello" }]
    });
    assert!(!detect_is_anthropic(&body));
}

#[test]
fn test_detect_is_anthropic_array_content_returns_true() {
    // Anthropic: content is an array
    let body = json!({
        "model": "mimo-7b",
        "messages": [{ "role": "user", "content": [
            { "type": "text", "text": "Hello" }
        ] }]
    });
    assert!(detect_is_anthropic(&body));
}

#[test]
fn test_detect_is_anthropic_system_field_returns_true() {
    // Anthropic: has top-level system field
    let body = json!({
        "model": "mimo-7b",
        "system": "You are helpful.",
        "messages": [{ "role": "user", "content": "Hi" }]
    });
    assert!(detect_is_anthropic(&body));
}

#[test]
fn test_detect_is_anthropic_empty_messages_returns_false() {
    let body = json!({ "model": "mimo-7b", "messages": [] });
    assert!(!detect_is_anthropic(&body));
}

#[test]
fn test_detect_is_anthropic_no_messages_returns_false() {
    let body = json!({ "model": "mimo-7b" });
    assert!(!detect_is_anthropic(&body));
}

// ---------------------------------------------------------------------------
// URL routing tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_anthropic_routes_to_messages_endpoint() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "msg-route",
        "content": [{ "type": "text", "text": "routed" }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 1, "output_tokens": 1 }
    });

    // Only mock /messages — /chat/completions is NOT mocked.
    // If routing is wrong, this test will fail with a connection error.
    let m = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(response_body.to_string())
        .create_async()
        .await;

    let provider = MimoProvider::with_base_url("sk-test".into(), &provider_url(&server));
    let req = make_request("mimo-7b");
    let body = json!({
        "model": "mimo-7b",
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Hi" }] }],
        "max_tokens": 100
    });

    let resp = provider.send(req, body).await.expect("send should succeed");
    m.assert_async().await;
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        crate::types::RawContentBlock::Text("routed".into())
    );
}

#[tokio::test]
async fn test_send_openai_routes_to_chat_endpoint() {
    let mut server = mockito::Server::new_async().await;

    let response_body = json!({
        "id": "mimo-001",
        "model": "mimo-7b",
        "choices": [{
            "message": { "role": "assistant", "content": "Hello!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    });

    // Only mock /chat/completions — /messages is NOT mocked.
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
        "messages": [{ "role": "user", "content": "Hi" }]
    });

    let resp = provider.send(req, body).await.expect("send should succeed");
    m.assert_async().await;
    assert_eq!(resp.content_blocks.len(), 1);
}

// ---------------------------------------------------------------------------
// supported_protocols tests
// ---------------------------------------------------------------------------

#[test]
fn test_supported_protocols_contains_openai() {
    let provider = MimoProvider::new("sk-test".into());
    let protocols = provider.supported_protocols();
    assert!(
        protocols.iter().any(|p| p.as_str() == "openai"),
        "supported_protocols must contain 'openai'"
    );
}

#[test]
fn test_supported_protocols_contains_anthropic() {
    let provider = MimoProvider::new("sk-test".into());
    let protocols = provider.supported_protocols();
    assert!(
        protocols.iter().any(|p| p.as_str() == "anthropic"),
        "supported_protocols must contain 'anthropic'"
    );
}
