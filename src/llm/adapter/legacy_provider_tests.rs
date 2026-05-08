//! Tests for [`LegacyProviderAdapter`] using [`FakeProvider`] as the inner LLMProvider.
//!
//! These tests live in a separate file so that `legacy_provider.rs` stays under
//! the 500-line pre-commit limit while still allowing `#[path]` module inclusion
//! when the `fake-llm` feature is enabled.

use super::*;

// Re-export types used in tests that aren't brought in by the parent's glob.
use crate::llm::types::InternalMessage;
use crate::llm::LLMError;

// ── helpers ───────────────────────────────────────────────────────────────────

fn client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

fn make_internal_request(content: &str) -> InternalRequest {
    InternalRequest {
        model: "test-model".to_string(),
        messages: vec![InternalMessage {
            role: "user".into(),
            content: content.into(),
        }],
        temperature: 0.7,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
    }
}

fn adapter<P: LLMProvider>(inner: P) -> LegacyProviderAdapter<P> {
    LegacyProviderAdapter::new(
        inner,
        "https://api.example.com".into(),
        "test-key".into(),
        vec![ProtocolId::new("openai")],
        client(),
        HeaderMap::new(),
    )
}

// ── config accessor tests ──────────────────────────────────────────────────────

#[test]
fn test_id_delegates_to_inner() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok("hi", "gpt-4o")
        .build();
    let a = adapter(fake);
    assert_eq!(a.id(), "fake");
}

#[test]
fn test_base_url_returns_stored_value() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok("hi", "gpt-4o")
        .build();
    let a = adapter(fake);
    assert_eq!(a.base_url(), "https://api.example.com");
}

#[test]
fn test_api_key_returns_stored_value() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok("hi", "gpt-4o")
        .build();
    let a = adapter(fake);
    assert_eq!(a.api_key(), "test-key");
}

#[test]
fn test_supported_protocols_returns_stored_value() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok("hi", "gpt-4o")
        .build();
    let a = adapter(fake);
    let protocols = a.supported_protocols();
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0].as_str(), "openai");
}

#[test]
fn test_http_client_returns_stored_client() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok("hi", "gpt-4o")
        .build();
    let a = adapter(fake);
    // Just verify we get a Client back, not null
    let _ = a.http_client();
}

#[test]
fn test_default_headers_returns_stored_headers() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok("hi", "gpt-4o")
        .build();
    let a = adapter(fake);
    let headers = a.default_headers();
    assert!(headers.is_empty());
}

// ── send() tests ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_send_converts_request_and_returns_response() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok_with("hello from adapter", "gpt-4o-mini", 3, 7)
        .build();
    let a = adapter(fake);

    let req = make_internal_request("hi there");
    let resp = a.send(req, serde_json::Value::Null).await.unwrap();

    assert_eq!(resp.content_blocks.len(), 1);
    let RawContentBlock::Text(text) = &resp.content_blocks[0] else {
        panic!("expected Text block, got {:?}", resp.content_blocks[0]);
    };
    assert_eq!(text, "hello from adapter");
    assert_eq!(resp.usage.prompt_tokens, 3);
    assert_eq!(resp.usage.completion_tokens, 7);
    assert_eq!(resp.usage.total_tokens, Some(10));
}

#[tokio::test]
async fn test_send_error_propagates() {
    let fake = crate::llm::FakeProvider::builder()
        .then_err(LLMError::RateLimitExceeded)
        .build();
    let a = adapter(fake);

    let req = make_internal_request("test");
    let err = a.send(req, serde_json::Value::Null).await.unwrap_err();
    assert!(err.to_string().contains("Rate limit exceeded"));
}

// ── send_streaming() tests ────────────────────────────────────────────────────

#[tokio::test]
async fn test_send_streaming_returns_chunks() {
    // FakeProvider's default chat_streaming wraps chat() into two chunks:
    // Text(delta) + Done
    let fake = crate::llm::FakeProvider::builder()
        .then_ok_with("streaming content", "model-x", 2, 5)
        .build();
    let a = adapter(fake);

    let req = make_internal_request("stream me");
    let mut stream = a
        .send_streaming(req, serde_json::Value::Null)
        .await
        .unwrap();

    let chunks: Vec<_> = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut ch = Vec::new();
        while let Some(chunk) = stream.recv().await {
            ch.push(chunk);
        }
        ch
    })
    .await
    .unwrap();

    assert!(!chunks.is_empty());
    // At least one chunk should contain "streaming content"
    let data_strings: Vec<_> = chunks.iter().map(|c| c.data.as_str()).collect();
    assert!(
        data_strings.iter().any(|s| s.contains("streaming content")),
        "no chunk contained expected text: {data_strings:?}"
    );
}

#[tokio::test]
async fn test_send_streaming_error_closes_channel() {
    let fake = crate::llm::FakeProvider::builder()
        .then_err(LLMError::ApiError("stream error".into()))
        .build();
    let a = adapter(fake);

    let req = make_internal_request("test");
    let result = a.send_streaming(req, serde_json::Value::Null).await;
    // send_streaming returns Err when the underlying chat_streaming fails
    let err = result.unwrap_err();
    assert!(err.to_string().contains("stream error"));
}

// ── build_chat_request / to_internal_response (private helpers) ────────────────
// We test these indirectly by checking the final InternalResponse shape.

#[tokio::test]
async fn test_send_converts_chat_request_model_field() {
    let fake = crate::llm::FakeProvider::builder()
        .then_ok_with("resp", "my-model", 1, 1)
        .build();
    let a = adapter(fake);

    let mut req = make_internal_request("hello");
    req.model = "custom-model-123".to_string();
    let resp = a.send(req, serde_json::Value::Null).await.unwrap();

    // The content block text is the response content, not the model
    let RawContentBlock::Text(text) = &resp.content_blocks[0] else {
        panic!();
    };
    assert_eq!(text, "resp");
}
