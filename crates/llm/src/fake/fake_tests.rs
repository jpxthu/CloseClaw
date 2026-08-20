use super::super::provider::{Provider, ProviderError};
use super::super::types::{InternalMessage, InternalRequest, ProtocolId, RawContentBlock};
use super::fake_scenario::DeliveryConfig;
use super::*;
use closeclaw_session::persistence::ReasoningLevel;
use std::time::{Duration, Instant};

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "test-model".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            ..Default::default()
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
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
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

    // OpenAI SSE: role chunk → content chunk → finish chunk → [DONE]
    let role_chunk = rx.recv().await.unwrap();
    let role_data: serde_json::Value = serde_json::from_str(&role_chunk.data).unwrap();
    assert_eq!(role_data["choices"][0]["delta"]["role"], "assistant");
    assert!(role_data["choices"][0]["delta"]["content"].is_null());

    let content_chunk = rx.recv().await.unwrap();
    let content_data: serde_json::Value = serde_json::from_str(&content_chunk.data).unwrap();
    assert_eq!(
        content_data["choices"][0]["delta"]["content"],
        "streamed content"
    );

    let finish_chunk = rx.recv().await.unwrap();
    let finish_data: serde_json::Value = serde_json::from_str(&finish_chunk.data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "stop");

    let done_chunk = rx.recv().await.unwrap();
    assert_eq!(done_chunk.data, "[DONE]");

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

    // OpenAI SSE: role chunk → content chunk → finish chunk → [DONE]
    let role_chunk = rx.recv().await.unwrap();
    let role_data: serde_json::Value = serde_json::from_str(&role_chunk.data).unwrap();
    assert_eq!(role_data["choices"][0]["delta"]["role"], "assistant");

    let content_chunk = rx.recv().await.unwrap();
    let content_data: serde_json::Value = serde_json::from_str(&content_chunk.data).unwrap();
    assert_eq!(
        content_data["choices"][0]["delta"]["content"],
        "delayed stream"
    );

    let finish_chunk = rx.recv().await.unwrap();
    let finish_data: serde_json::Value = serde_json::from_str(&finish_chunk.data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "stop");

    let done_chunk = rx.recv().await.unwrap();
    assert_eq!(done_chunk.data, "[DONE]");

    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn test_send_streaming_fallback() {
    let provider = FakeProvider::builder().or_else("fallback stream").build();

    let mut rx = provider
        .send_streaming(make_request(), serde_json::Value::Null)
        .await
        .unwrap();

    // Fallback uses OpenAI SSE format via generate_openai_sse
    // role chunk → content chunk → finish chunk → [DONE]
    let role_chunk = rx.recv().await.unwrap();
    let role_data: serde_json::Value = serde_json::from_str(&role_chunk.data).unwrap();
    assert_eq!(role_data["choices"][0]["delta"]["role"], "assistant");

    let content_chunk = rx.recv().await.unwrap();
    let content_data: serde_json::Value = serde_json::from_str(&content_chunk.data).unwrap();
    assert_eq!(
        content_data["choices"][0]["delta"]["content"],
        "fallback stream"
    );

    let finish_chunk = rx.recv().await.unwrap();
    let finish_data: serde_json::Value = serde_json::from_str(&finish_chunk.data).unwrap();
    assert_eq!(finish_data["choices"][0]["finish_reason"], "stop");

    let done_chunk = rx.recv().await.unwrap();
    assert_eq!(done_chunk.data, "[DONE]");

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

// ── Cache fields in Scenario::Ok / Scenario::Err ──────────────────────────

#[tokio::test]
async fn test_ok_with_cache_raw_usage() {
    let scenario = Scenario::Ok {
        content_blocks: vec![RawContentBlock::Text("cached response".into())],
        model: "gpt-4".into(),
        prompt_tokens: 100,
        completion_tokens: 50,
        cache_read_tokens: Some(80),
        cache_write_tokens: Some(20),
        delivery: DeliveryConfig::default(),
        include_usage: false,
        protocol: ProtocolId::new("openai"),
        segment_granularity: 0,
    };
    let usage = scenario.raw_usage();
    assert_eq!(usage.prompt_tokens, 100);
    assert_eq!(usage.completion_tokens, 50);
    assert_eq!(usage.total_tokens, Some(150));
    assert_eq!(usage.cache_read_tokens, Some(80));
    assert_eq!(usage.cache_write_tokens, Some(20));

    // Also verify through the full Provider::send path
    let provider = FakeProvider::builder()
        .then_ok_with_cache("cached", "gpt-4", 200, 80, (Some(150), Some(30)))
        .build();
    let resp = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(resp.usage.prompt_tokens, 200);
    assert_eq!(resp.usage.completion_tokens, 80);
    assert_eq!(resp.usage.cache_read_tokens, Some(150));
    assert_eq!(resp.usage.cache_write_tokens, Some(30));
}

#[tokio::test]
async fn test_ok_backward_compat_cache_none() {
    // Scenario::ok() should produce cache fields as None
    let scenario = Scenario::ok("hello", "model");
    let usage = scenario.raw_usage();
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.cache_write_tokens, None);
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 10);
    assert_eq!(usage.total_tokens, Some(20));

    // Verify through the full Provider::send path
    let provider = FakeProvider::builder().then_ok("hello", "model").build();
    let resp = provider
        .send(make_request(), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(resp.usage.cache_read_tokens, None);
    assert_eq!(resp.usage.cache_write_tokens, None);
}

#[tokio::test]
async fn test_err_scenario_cache_fields_none() {
    // Scenario::Err raw_usage() should always have cache fields as None
    let scenario = Scenario::err(ProviderError::Legacy("test error".into()));
    let usage = scenario.raw_usage();
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.cache_write_tokens, None);
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 0);

    // Also test with custom usage via err_with
    let scenario_with_usage =
        Scenario::err_with(ProviderError::Legacy("rate limit".into()), 50, 25);
    let usage2 = scenario_with_usage.raw_usage();
    assert_eq!(usage2.cache_read_tokens, None);
    assert_eq!(usage2.cache_write_tokens, None);
    assert_eq!(usage2.prompt_tokens, 50);
    assert_eq!(usage2.completion_tokens, 25);
}

// ── Builder API convenience methods (Step 1.5) ─────────────────────────

#[test]
fn test_builder_then_streaming() {
    let provider = FakeProvider::builder()
        .then_streaming("stream me", "model-a", ProtocolId::new("anthropic"), 3)
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok {
            content_blocks,
            model,
            protocol,
            segment_granularity,
            delivery,
            include_usage,
            ..
        } => {
            assert_eq!(
                content_blocks,
                vec![RawContentBlock::Text("stream me".into())]
            );
            assert_eq!(model, "model-a");
            assert_eq!(protocol, ProtocolId::new("anthropic"));
            assert_eq!(segment_granularity, 3);
            assert!(delivery.first_token_delay.is_none());
            assert!(!include_usage);
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_then_with_first_token_delay() {
    let provider = FakeProvider::builder()
        .then_with_first_token_delay("content", "model-b", Duration::from_millis(200))
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok {
            content_blocks,
            delivery,
            protocol,
            ..
        } => {
            assert_eq!(
                content_blocks,
                vec![RawContentBlock::Text("content".into())]
            );
            assert_eq!(delivery.first_token_delay, Some(Duration::from_millis(200)));
            assert!(delivery.per_segment_delay.is_none());
            assert!(delivery.overall_delay.is_none());
            assert_eq!(protocol, ProtocolId::new("openai"));
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_then_with_per_segment_delay() {
    let provider = FakeProvider::builder()
        .then_with_per_segment_delay("content", "model-c", Duration::from_millis(50))
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok {
            content_blocks,
            delivery,
            ..
        } => {
            assert_eq!(
                content_blocks,
                vec![RawContentBlock::Text("content".into())]
            );
            assert!(delivery.first_token_delay.is_none());
            assert_eq!(delivery.per_segment_delay, Some(Duration::from_millis(50)));
            assert!(delivery.overall_delay.is_none());
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_then_with_overall_delay() {
    let provider = FakeProvider::builder()
        .then_with_overall_delay("content", "model-d", Duration::from_millis(100))
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok {
            content_blocks,
            delivery,
            ..
        } => {
            assert_eq!(
                content_blocks,
                vec![RawContentBlock::Text("content".into())]
            );
            assert!(delivery.first_token_delay.is_none());
            assert!(delivery.per_segment_delay.is_none());
            assert_eq!(delivery.overall_delay, Some(Duration::from_millis(100)));
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_then_http_error() {
    let provider = FakeProvider::builder()
        .then_http_error("content", "model-e", 429, Some(30))
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok {
            content_blocks,
            delivery,
            prompt_tokens,
            completion_tokens,
            ..
        } => {
            assert_eq!(
                content_blocks,
                vec![RawContentBlock::Text("content".into())]
            );
            let ei = delivery.error_injection.unwrap();
            assert_eq!(ei.status_code, 429);
            assert_eq!(ei.message, "HTTP 429");
            assert_eq!(ei.retry_after, Some(30));
            assert_eq!(prompt_tokens, 0);
            assert_eq!(completion_tokens, 0);
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_then_http_error_no_retry() {
    let provider = FakeProvider::builder()
        .then_http_error("content", "model-f", 500, None)
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok { delivery, .. } => {
            let ei = delivery.error_injection.unwrap();
            assert_eq!(ei.status_code, 500);
            assert!(ei.retry_after.is_none());
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_then_stream_interrupt() {
    let provider = FakeProvider::builder()
        .then_stream_interrupt("content", "model-g", 5)
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok {
            content_blocks,
            delivery,
            ..
        } => {
            assert_eq!(
                content_blocks,
                vec![RawContentBlock::Text("content".into())]
            );
            let si = delivery.stream_interrupt.unwrap();
            assert_eq!(si.interrupt_after_frames, 5);
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_include_usage() {
    let provider = FakeProvider::builder()
        .then_ok("content", "model-h")
        .include_usage(true)
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Ok { include_usage, .. } => {
            assert!(include_usage);
        }
        _ => panic!("Expected Scenario::Ok"),
    }
}

#[test]
fn test_builder_include_usage_noop_on_err() {
    // include_usage should be a no-op when last scenario is not Scenario::Ok
    let provider = FakeProvider::builder()
        .then_err(ProviderError::Legacy("err".into()))
        .include_usage(true)
        .build();

    let scenario = provider
        .inner
        .lock()
        .unwrap()
        .scenarios
        .pop_front()
        .unwrap();
    match scenario {
        Scenario::Err { .. } => {}
        _ => panic!("Expected Scenario::Err"),
    }
}

#[test]
fn test_builder_include_usage_on_last_only() {
    // Only the last scenario's include_usage should be set
    let provider = FakeProvider::builder()
        .then_ok("first", "m")
        .then_ok("second", "m")
        .include_usage(true)
        .build();

    let state = provider.inner.lock().unwrap();
    let scenarios: Vec<_> = state.scenarios.iter().collect();
    match scenarios[0] {
        Scenario::Ok { include_usage, .. } => assert!(!include_usage),
        _ => panic!("Expected Scenario::Ok"),
    }
    match scenarios[1] {
        Scenario::Ok { include_usage, .. } => assert!(include_usage),
        _ => panic!("Expected Scenario::Ok"),
    }
}
