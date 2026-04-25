use super::*;
use std::time::Instant;

fn make_request() -> ChatRequest {
    ChatRequest {
        model: "test-model".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "hello".to_string(),
        }],
        temperature: 0.7,
        max_tokens: None,
    }
}

#[tokio::test]
async fn test_ok_scenario() {
    let provider = FakeProvider::builder()
        .then_ok_with("hello world", "gpt-4", 5, 12)
        .build();

    let resp = provider.chat(make_request()).await.unwrap();
    assert_eq!(resp.content, "hello world");
    assert_eq!(resp.model, "gpt-4");
    assert_eq!(resp.usage.prompt_tokens, 5);
    assert_eq!(resp.usage.completion_tokens, 12);
    assert_eq!(resp.usage.total_tokens, 17);
}

#[tokio::test]
async fn test_err_scenario() {
    let provider = FakeProvider::builder()
        .then_err(LLMError::RateLimitExceeded)
        .build();

    let err = provider.chat(make_request()).await.unwrap_err();
    assert!(matches!(err, LLMError::RateLimitExceeded));
}

#[tokio::test]
async fn test_sequential_scenarios() {
    let provider = FakeProvider::builder()
        .then_ok("first", "model-a")
        .then_ok("second", "model-b")
        .then_err(LLMError::AuthFailed("bad".to_string()))
        .build();

    let resp1 = provider.chat(make_request()).await.unwrap();
    assert_eq!(resp1.content, "first");
    assert_eq!(resp1.model, "model-a");

    let resp2 = provider.chat(make_request()).await.unwrap();
    assert_eq!(resp2.content, "second");
    assert_eq!(resp2.model, "model-b");

    let err = provider.chat(make_request()).await.unwrap_err();
    assert!(matches!(err, LLMError::AuthFailed(_)));
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
    let resp = delayed_provider.chat(make_request()).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.content, "delayed");
    assert!(elapsed.as_millis() >= 180, "delay too short: {elapsed:?}");
    assert!(elapsed.as_millis() <= 600, "delay too long: {elapsed:?}");
}

#[tokio::test]
async fn test_request_captured() {
    let provider = FakeProvider::builder()
        .then_ok("resp", "model-x")
        .then_ok("resp2", "model-y")
        .build();

    provider.chat(make_request()).await.unwrap();
    provider.chat(make_request()).await.unwrap();

    let captured = provider.captured_requests();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].model, "test-model");
    assert_eq!(captured[0].messages[0].content, "hello");
    assert_eq!(captured[1].model, "test-model");
}

#[tokio::test]
#[should_panic(expected = "scenarios exhausted")]
async fn test_exhaust_panics() {
    let provider = FakeProvider::builder().then_ok("only one", "model").build();

    provider.chat(make_request()).await.unwrap();
    provider.chat(make_request()).await.unwrap();
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

    let resp1 = provider.chat(make_request()).await.unwrap();
    assert_eq!(resp1.content, "first");

    let resp2 = provider.chat(make_request()).await.unwrap();
    assert_eq!(resp2.content, "fallback response");
    assert_eq!(resp2.model, "fake-model");

    let resp3 = provider.chat(make_request()).await.unwrap();
    assert_eq!(resp3.content, "fallback response");
}
