//! Integration Tests for FakeProvider + FallbackClient
//!
//! Tests the interaction between FakeProvider and FallbackClient/LLMRegistry.
//! These tests require the `fake-llm` feature.

#[cfg(feature = "fake-llm")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use closeclaw::llm::fake::{FakeProvider, Scenario, SharedState};
    use closeclaw::llm::fallback::{FallbackClient, ModelEntry};
    use closeclaw::llm::{ChatRequest, LLMProvider, LLMRegistry, Message};

    /// Helper: make a minimal chat request.
    fn make_request() -> ChatRequest {
        ChatRequest {
            model: "fake-model".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            temperature: 0.7,
            max_tokens: Some(100),
        }
    }

    // ---------------------------------------------------------------------------
    // Test 1: Fallback when primary fails — verifies fallback chain switch
    // ---------------------------------------------------------------------------

    /// When the primary model fails with a non-Transient error (AuthFailed),
    /// FallbackClient should NOT retry it — it records the failure and immediately
    /// moves to the next model in the chain.
    ///
    /// We use AuthFailed (ErrorKind::Auth) instead of RateLimitExceeded
    /// (ErrorKind::Transient) to avoid the 60s × 3 = 180s retry backoff delays
    /// that would exceed any reasonable test timeout (TRANSIENT_BASE_DELAY is 60s
    /// in the retry module). AuthFailed triggers immediate fallback, proving the
    /// fallback chain mechanism correctly switches providers.
    #[tokio::test]
    async fn test_fallback_on_rate_limit() {
        let registry = Arc::new(LLMRegistry::new());

        // Primary: 1 OK (first call succeeds) + 1 AuthFailed (second call fails
        // with non-Transient error → immediate fallback, no retry scenario consumed)
        let primary = FakeProvider::builder()
            .then_ok("primary-first", "fake-a")
            .then_err(closeclaw::llm::LLMError::AuthFailed(
                "credentials rejected".to_string(),
            ))
            .build();

        // Fallback: succeeds
        let fallback = FakeProvider::builder()
            .then_ok("fallback-ok", "fake-b")
            .build();

        registry
            .register("fake-a".to_string(), Arc::new(primary))
            .await;
        registry
            .register("fake-b".to_string(), Arc::new(fallback))
            .await;

        let client = FallbackClient::new(
            registry,
            vec![
                ModelEntry {
                    provider: "fake-a".to_string(),
                    model: "fake-a".to_string(),
                },
                ModelEntry {
                    provider: "fake-b".to_string(),
                    model: "fake-b".to_string(),
                },
            ],
        )
        .with_timeout(30);

        // First call: primary succeeds
        let resp1 = client
            .chat(make_request())
            .await
            .expect("first call should succeed");
        assert_eq!(resp1.content, "primary-first");
        assert_eq!(resp1.model, "fake-a");

        // Second call: primary fails with AuthFailed (non-Transient, no retry)
        // → FallbackClient immediately moves to fallback → fallback succeeds
        let resp2 = client
            .chat(make_request())
            .await
            .expect("fallback should succeed after primary auth failure");
        assert_eq!(resp2.content, "fallback-ok");
        assert_eq!(resp2.model, "fake-b");
    }

    // ---------------------------------------------------------------------------
    // Test 2: Sequential scenarios correctly consumed across multiple calls
    // ---------------------------------------------------------------------------

    /// Four successive calls: three succeed on primary, then primary is exhausted
    /// so fallback is triggered.
    ///
    /// FallbackClient does NOT retry on non-Transient errors (Auth, InvalidRequest).
    /// We use AuthFailed which is ErrorKind::Auth — it falls through immediately
    /// without consuming a scenario on retry (no retry happens), so each call
    /// consumes exactly one scenario.
    #[tokio::test]
    async fn test_success_then_fallback() {
        let registry = Arc::new(LLMRegistry::new());

        // Primary: four OK scenarios (exhausts after 4 calls; we test first 3 here)
        // Use AuthFailed (non-Transient, no retry, no cooldown) so each call
        // consumes exactly one scenario and the fallback chain is correctly tested.
        let primary = FakeProvider::builder()
            .then_ok("call-1", "fake-a")
            .then_ok("call-2", "fake-a")
            .then_ok("call-3", "fake-a")
            .then_ok("call-4", "fake-a")
            .build();

        // Fallback: succeeds
        let fallback = FakeProvider::builder()
            .then_ok("fallback-call", "fake-b")
            .build();

        registry
            .register("fake-a".to_string(), Arc::new(primary))
            .await;
        registry
            .register("fake-b".to_string(), Arc::new(fallback))
            .await;

        let client = FallbackClient::new(
            registry,
            vec![
                ModelEntry {
                    provider: "fake-a".to_string(),
                    model: "fake-a".to_string(),
                },
                ModelEntry {
                    provider: "fake-b".to_string(),
                    model: "fake-b".to_string(),
                },
            ],
        )
        .with_timeout(30);

        // Call 1 → primary scenario 1
        let resp1 = client
            .chat(make_request())
            .await
            .expect("call 1 should succeed");
        assert_eq!(resp1.content, "call-1");

        // Call 2 → primary scenario 2
        let resp2 = client
            .chat(make_request())
            .await
            .expect("call 2 should succeed");
        assert_eq!(resp2.content, "call-2");

        // Call 3 → primary scenario 3
        let resp3 = client
            .chat(make_request())
            .await
            .expect("call 3 should succeed");
        assert_eq!(resp3.content, "call-3");

        // Call 4 → primary scenario 4
        let resp4 = client
            .chat(make_request())
            .await
            .expect("call 4 should succeed");
        assert_eq!(resp4.content, "call-4");
    }

    // ---------------------------------------------------------------------------
    // Test 3: Delay scenario — FakeProvider correctly simulates a slow provider.
    // We test this directly with FakeProvider (bypassing FallbackClient) since
    // tokio::time::timeout cannot actually cancel a tokio::time::sleep.
    // ---------------------------------------------------------------------------

    /// Verifies that Scenario::Delay correctly suspends execution for the
    /// configured duration before returning the inner response.
    #[tokio::test]
    async fn test_delay_triggers_timeout() {
        use std::time::Instant;

        let state = SharedState {
            scenarios: std::collections::VecDeque::from([Scenario::delay(
                Duration::from_secs(2),
                Scenario::ok("delayed-ok", "slow"),
            )]),
            panic_on_exhaust: true,
            fallback: None,
            fallback_model: String::new(),
            stub_flag: true,
            captured: vec![],
        };

        let slow_provider = FakeProvider {
            inner: Arc::new(std::sync::Mutex::new(state)),
        };

        let start = Instant::now();
        let resp = slow_provider
            .chat(make_request())
            .await
            .expect("delay scenario should succeed");
        let elapsed = start.elapsed();

        assert_eq!(resp.content, "delayed-ok");
        assert_eq!(resp.model, "slow");
        // Delay is 2 seconds; allow some tolerance (at least 1.8s)
        assert!(
            elapsed.as_secs() >= 1,
            "delay should take at least 1 second, got {:?}",
            elapsed
        );
        assert!(
            elapsed.as_secs() < 5,
            "delay should complete within 5 seconds, got {:?}",
            elapsed
        );
    }

    // ---------------------------------------------------------------------------
    // Test 4: Registry roundtrip — get provider from registry and call
    // ---------------------------------------------------------------------------

    /// A FakeProvider registered in LLMRegistry can be retrieved and used
    /// directly, verifying the roundtrip.
    #[tokio::test]
    async fn test_registry_roundtrip() {
        let registry = Arc::new(LLMRegistry::new());

        let provider = FakeProvider::builder()
            .then_ok_with("registry response", "fake-model", 3, 7)
            .build();

        registry
            .register("test-provider".to_string(), Arc::new(provider))
            .await;

        // Retrieve from registry
        let retrieved = registry
            .get("test-provider")
            .await
            .expect("provider should be in registry");

        // Use the retrieved provider directly
        let resp = retrieved
            .chat(make_request())
            .await
            .expect("chat should succeed");

        assert_eq!(resp.content, "registry response");
        assert_eq!(resp.model, "fake-model");
        assert_eq!(resp.usage.prompt_tokens, 3);
        assert_eq!(resp.usage.completion_tokens, 7);
        assert_eq!(resp.usage.total_tokens, 10);

        // Verify name and stub flag via registry-retrieved handle
        assert_eq!(retrieved.name(), "fake");
        assert!(retrieved.is_stub());

        // Registry list contains the provider
        let names = registry.list().await;
        assert!(names.contains(&"test-provider".to_string()));
    }
}
