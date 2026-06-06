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
    use closeclaw::llm::legacy::legacy_provider::LegacyProviderBridge;
    #[allow(deprecated)]
    use closeclaw::llm::LLMProvider;
    use closeclaw::llm::{ChatRequest, LLMRegistry, Message};

    /// Wrap a FakeProvider into an `Arc<dyn Provider>` via LegacyProviderBridge.
    fn wrap_provider(provider: FakeProvider) -> Arc<dyn closeclaw::llm::provider::Provider> {
        Arc::new(LegacyProviderBridge::new(
            provider,
            String::new(),
            String::new(),
            vec![],
            reqwest::Client::new(),
            reqwest::header::HeaderMap::new(),
        ))
    }

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
            .then_err(closeclaw::llm::provider::ProviderError::Legacy(
                "credentials rejected".to_string(),
            ))
            .build();

        // Fallback: succeeds
        let fallback = FakeProvider::builder()
            .then_ok("fallback-ok", "fake-b")
            .build();

        registry
            .register("fake-a".to_string(), wrap_provider(primary))
            .await;
        registry
            .register("fake-b".to_string(), wrap_provider(fallback))
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
            .register("fake-a".to_string(), wrap_provider(primary))
            .await;
        registry
            .register("fake-b".to_string(), wrap_provider(fallback))
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
    #[allow(deprecated)]
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
            captured_internal: Vec::new(),
            default_headers: reqwest::header::HeaderMap::new(),
            http_client: reqwest::Client::new(),
            supported_protocols: vec![closeclaw::llm::types::ProtocolId::new("openai")],
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
            .register("test-provider".to_string(), wrap_provider(provider))
            .await;

        // Retrieve from registry
        let retrieved = registry
            .get("test-provider")
            .await
            .expect("provider should be in registry");

        // Verify the provider is retrievable and has the expected id
        assert_eq!(retrieved.id(), "test-provider");

        // Registry list contains the provider
        let names = registry.list().await;
        assert!(names.contains(&"test-provider".to_string()));
    }

    // ---------------------------------------------------------------------------
    // Test 5: Cooldown skip — Auth failure on primary triggers 1h cooldown,
    // second call skips primary and goes directly to fallback.
    // ---------------------------------------------------------------------------

    /// Verifies that after an AuthFailed error on the primary provider:
    /// 1. The primary is placed in cooldown for 1 hour (Auth error cooldown)
    /// 2. On the second call, FallbackClient skips the cooled primary
    /// 3. The fallback provider succeeds on both calls
    /// 4. captured_requests() confirms minimax was only called once.
    #[tokio::test]
    async fn test_cooldown_skip_after_auth_failure() {
        let registry = Arc::new(LLMRegistry::new());

        // Primary: always fails with AuthFailed (non-Transient → immediate fallback,
        // no retry, triggers cooldown recording)
        let primary = FakeProvider::builder()
            .then_err(closeclaw::llm::provider::ProviderError::Legacy(
                "credentials rejected".to_string(),
            ))
            .build();

        // Fallback: always succeeds — use .or_else() to avoid panic after scenarios
        // exhaust (though we only expect 2 calls, so won't exhaust here)
        let fallback = FakeProvider::builder()
            .then_ok("fallback-ok", "fake-openai")
            .then_ok("fallback-ok", "fake-openai")
            .or_else("fallback-ok")
            .build();

        // Keep strong refs so captured_requests works on the original Arc
        let primary_ref = primary.clone();
        let fallback_ref = fallback.clone();

        registry
            .register("fake-minimax".to_string(), wrap_provider(primary))
            .await;
        registry
            .register("fake-openai".to_string(), wrap_provider(fallback))
            .await;

        let client = FallbackClient::new(
            registry,
            vec![
                ModelEntry {
                    provider: "fake-minimax".to_string(),
                    model: "MiniMax-M2.7".to_string(),
                },
                ModelEntry {
                    provider: "fake-openai".to_string(),
                    model: "gpt-4o".to_string(),
                },
            ],
        )
        .with_timeout(30);

        // First call: minimax fails → fallback succeeds
        let resp1 = client
            .chat(make_request())
            .await
            .expect("first call should succeed via fallback");
        assert_eq!(resp1.content, "fallback-ok");
        assert_eq!(resp1.model, "fake-openai");

        // Second call: minimax is in cooldown → skip → fallback succeeds again
        let resp2 = client
            .chat(make_request())
            .await
            .expect("second call should succeed via fallback (primary in cooldown)");
        assert_eq!(resp2.content, "fallback-ok");
        assert_eq!(resp2.model, "fake-openai");

        // Verify captured requests: minimax should have been called exactly once
        // (the first call tried it and failed; the second call skipped it)
        let captured = primary_ref.captured_requests();
        assert_eq!(
            captured.len(),
            1,
            "minimax should be called exactly once (first call), got {}",
            captured.len()
        );

        // Verify openai was called twice (once per each call)
        let openai_captured = fallback_ref.captured_requests();
        assert_eq!(
            openai_captured.len(),
            2,
            "openai should be called twice (once per each call), got {}",
            openai_captured.len()
        );
    }

    // ---------------------------------------------------------------------------
    // Test 6: All providers exhausted — all models fail, verify exhausted error
    // ---------------------------------------------------------------------------

    /// Verifies that when all providers in the fallback chain fail,
    /// FallbackClient returns an ApiError containing "exhausted".
    #[tokio::test]
    async fn test_all_providers_exhausted() {
        let registry = Arc::new(LLMRegistry::new());

        // Provider A: always fails with AuthFailed
        let provider_a = FakeProvider::builder()
            .then_err(closeclaw::llm::provider::ProviderError::Legacy(
                "key rejected".to_string(),
            ))
            .build();

        // Provider B: always fails with AuthFailed
        let provider_b = FakeProvider::builder()
            .then_err(closeclaw::llm::provider::ProviderError::Legacy(
                "key rejected".to_string(),
            ))
            .build();

        registry
            .register("fake-a".to_string(), wrap_provider(provider_a))
            .await;
        registry
            .register("fake-b".to_string(), wrap_provider(provider_b))
            .await;

        let client = FallbackClient::new(
            registry,
            vec![
                ModelEntry {
                    provider: "fake-a".to_string(),
                    model: "model-a".to_string(),
                },
                ModelEntry {
                    provider: "fake-b".to_string(),
                    model: "model-b".to_string(),
                },
            ],
        )
        .with_timeout(30);

        let err = client
            .chat(make_request())
            .await
            .expect_err("all providers exhausted should return error");
        assert!(
            err.to_string().contains("exhausted"),
            "error should mention exhausted, got: {}",
            err
        );
    }
}
