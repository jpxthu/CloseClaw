use super::super::provider::{Provider, ProviderError};
use super::super::types::{InternalMessage, InternalRequest, ProtocolId, RawContentBlock};
use super::*;
use crate::session::persistence::ReasoningLevel;
use std::time::Instant;

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "test-model".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        }],
        temperature: 0.7,
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

fn extract_text(resp: &InternalResponse) -> &str {
    match &resp.content_blocks[0] {
        RawContentBlock::Text(s) => s.as_str(),
        other => panic!("Expected Text block, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_ok_scenario() {
    let provider = FakeProvider::builder()
        .then_ok_with("hello world", "gpt-4", 5, 12)
        .build();

    let resp = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(extract_text(&resp), "hello world");
    assert_eq!(resp.usage.prompt_tokens, 5);
    assert_eq!(resp.usage.completion_tokens, 12);
    assert_eq!(resp.usage.total_tokens, Some(17));
}

#[tokio::test]
async fn test_err_scenario() {
    let provider = FakeProvider::builder()
        .then_err(ProviderError::Legacy("rate limit exceeded".into()))
        .build();

    let err = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Legacy(s) if s == "rate limit exceeded"));
}

#[tokio::test]
async fn test_sequential_scenarios() {
    let provider = FakeProvider::builder()
        .then_ok("first", "model-a")
        .then_ok("second", "model-b")
        .then_err(ProviderError::Legacy("auth failed".into()))
        .build();

    let resp1 = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(extract_text(&resp1), "first");

    let resp2 = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(extract_text(&resp2), "second");

    let err = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Legacy(s) if s == "auth failed"));
}

#[tokio::test]
async fn test_delay_simulated() {
    let delayed_provider = FakeProvider {
        inner: Arc::new(Mutex::new(super::SharedState {
            scenarios: std::collections::VecDeque::from([Scenario::Delay {
                duration: std::time::Duration::from_millis(200),
                inner: Box::new(Scenario::ok("delayed", "slow-model")),
            }]),
            ..Default::default()
        })),
    };

    let start = Instant::now();
    let resp = delayed_provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(extract_text(&resp), "delayed");
    assert!(elapsed.as_millis() >= 180, "delay too short: {elapsed:?}");
    assert!(elapsed.as_millis() <= 600, "delay too long: {elapsed:?}");
}

#[tokio::test]
async fn test_request_captured() {
    let provider = FakeProvider::builder()
        .then_ok("resp", "model-x")
        .then_ok("resp2", "model-y")
        .build();

    provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();

    let captured = provider.captured_internal_requests();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].request.model, "test-model");
    assert_eq!(captured[0].request.messages[0].content, "hello");
    assert_eq!(captured[1].request.model, "test-model");
}

#[tokio::test]
#[should_panic(expected = "scenarios exhausted")]
async fn test_exhaust_panics() {
    let provider = FakeProvider::builder().then_ok("only one", "model").build();

    provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_stub_flag() {
    let provider = FakeProvider::new();
    assert!(provider.is_stub());

    let provider = FakeProvider::builder().stub(false).build();
    assert!(!provider.is_stub());

    let provider = FakeProvider::builder().stub(true).build();
    assert!(provider.is_stub());
}

#[tokio::test]
async fn test_or_else_fallback() {
    let provider = FakeProvider::builder()
        .then_ok("first", "model")
        .or_else("fallback response")
        .build();

    let resp1 = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(extract_text(&resp1), "first");

    let resp2 = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(extract_text(&resp2), "fallback response");

    let resp3 = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(extract_text(&resp3), "fallback response");
}

#[tokio::test]
async fn test_send_streaming_ok() {
    let provider = FakeProvider::builder()
        .then_ok("streamed content", "model-s")
        .build();

    let mut rx = provider
        .send_streaming(make_request(), serde_json::Value::Null)
        .await
        .unwrap();

    let chunk1 = rx.recv().await.unwrap();
    assert_eq!(chunk1.event_type, "message");
    assert_eq!(chunk1.data, "streamed content");

    let chunk2 = rx.recv().await.unwrap();
    assert_eq!(chunk2.event_type, "message");
    assert!(chunk2.data.contains("message_end"));

    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn test_send_streaming_err() {
    let provider = FakeProvider::builder()
        .then_err(ProviderError::Legacy("stream error".into()))
        .build();

    let err = provider
        .send_streaming(make_request(), serde_json::Value::Null)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Legacy(s) if s == "stream error"));
}

#[tokio::test]
async fn test_send_streaming_delay() {
    let provider = FakeProvider::builder()
        .then_delay(
            std::time::Duration::from_millis(100),
            Scenario::ok("delayed stream", "delay-model"),
        )
        .build();

    let mut rx = provider
        .send_streaming(make_request(), serde_json::Value::Null)
        .await
        .unwrap();

    let chunk1 = rx.recv().await.unwrap();
    assert_eq!(chunk1.event_type, "message");
    assert_eq!(chunk1.data, "delayed stream");

    let chunk2 = rx.recv().await.unwrap();
    assert!(chunk2.data.contains("message_end"));

    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn test_send_streaming_fallback() {
    let provider = FakeProvider::builder().or_else("fallback stream").build();

    let mut rx = provider
        .send_streaming(make_request(), serde_json::Value::Null)
        .await
        .unwrap();

    let chunk1 = rx.recv().await.unwrap();
    assert_eq!(chunk1.event_type, "message");
    assert_eq!(chunk1.data, "fallback stream");

    let chunk2 = rx.recv().await.unwrap();
    assert!(chunk2.data.contains("message_end"));

    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn test_send_streaming_request_captured() {
    let provider = FakeProvider::builder().then_ok("ok", "m").build();

    let _ = provider
        .send_streaming(make_request(), serde_json::Value::Null)
        .await
        .unwrap();

    let captured = provider.captured_internal_requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].request.model, "test-model");
}

// ── Provider trait config accessors ───────────────────────────────────────

#[test]
fn test_provider_id() {
    let provider = FakeProvider::new();
    assert_eq!(Provider::id(&provider), "fake");
}

#[test]
fn test_provider_base_url() {
    let provider = FakeProvider::new();
    assert_eq!(Provider::base_url(&provider), "");
}

#[test]
fn test_provider_api_key() {
    let provider = FakeProvider::new();
    assert_eq!(Provider::api_key(&provider), "");
}

#[test]
fn test_provider_supported_protocols() {
    let provider = FakeProvider::new();
    let protocols = Provider::supported_protocols(&provider);
    assert!(
        protocols.contains(&ProtocolId::new("openai")),
        "expected 'openai' in protocols, got: {:?}",
        protocols
    );
}

#[test]
fn test_provider_http_client() {
    let provider = FakeProvider::new();
    let _client = Provider::http_client(&provider);
    // The Client was returned successfully — it's always valid.
    // We just verify the call doesn't panic and returns a reference.
}

#[test]
fn test_provider_default_headers() {
    let provider = FakeProvider::new();
    let headers = Provider::default_headers(&provider);
    assert!(
        headers.is_empty(),
        "expected empty HeaderMap, got: {:?}",
        headers
    );
}
