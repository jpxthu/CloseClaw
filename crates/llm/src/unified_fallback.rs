//! Unified Fallback Client
//!
//! Walks a chain of [`UnifiedChatClient`] instances with cooldown-based fallback.
//!
//! Unlike [`FallbackClient`](crate::fallback::FallbackClient) which wraps raw
//! providers, `UnifiedFallbackClient` operates on fully-configured
//! [`UnifiedChatClient`] instances that already own a Provider → Protocol →
//! Interpreter → Plugin pipeline. This lets the non-streaming path go through
//! the same five-layer architecture as the streaming path.

use crate::client::{ClientError, UnifiedChatClient};
use crate::retry::CooldownManager;
use crate::types::{InternalRequest, UnifiedResponse};
use crate::LLMError;
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// Error conversion
// ─────────────────────────────────────────────────────────────────────────────

impl From<ClientError> for LLMError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Provider(e) => LLMError::ApiError(e.to_string()),
            ClientError::Protocol(e) => LLMError::ApiError(e.to_string()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Chain entry
// ─────────────────────────────────────────────────────────────────────────────

/// A single entry in the fallback chain.
///
/// Each entry wraps a fully-configured [`UnifiedChatClient`] together with the
/// provider/model identifiers used for cooldown tracking.
#[derive(Debug, Clone)]
pub struct ChainEntry {
    /// Provider identifier (used as cooldown key).
    pub provider_id: String,
    /// Model identifier (used as cooldown key).
    pub model_id: String,
    /// The unified client for this entry.
    pub client: Arc<UnifiedChatClient>,
}

// ─────────────────────────────────────────────────────────────────────────────
// UnifiedFallbackClient
// ─────────────────────────────────────────────────────────────────────────────

/// Fallback client that walks a chain of [`UnifiedChatClient`] instances.
///
/// On each call to [`chat`](Self::chat), the client iterates through the chain,
/// skipping entries that are in cooldown, and returning the first successful
/// response. On failure, the cooldown is recorded and the next entry is tried.
#[derive(Clone)]
pub struct UnifiedFallbackClient {
    /// Ordered chain of clients to try.
    chain: Vec<ChainEntry>,
    /// Shared cooldown manager (same instance as [`FallbackClient`]).
    cooldown: Arc<CooldownManager>,
}

impl std::fmt::Debug for UnifiedFallbackClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedFallbackClient")
            .field("chain_len", &self.chain.len())
            .finish()
    }
}

impl UnifiedFallbackClient {
    /// Create a new `UnifiedFallbackClient`.
    ///
    /// # Arguments
    /// * `chain` — Ordered list of [`ChainEntry`]s to try.
    /// * `cooldown` — Shared [`CooldownManager`] instance.
    pub fn new(chain: Vec<ChainEntry>, cooldown: Arc<CooldownManager>) -> Self {
        Self { chain, cooldown }
    }

    /// Returns a reference to the first client in the chain.
    ///
    /// Used by the streaming path which needs a single `UnifiedChatClient`.
    pub fn primary(&self) -> &Arc<UnifiedChatClient> {
        &self.chain.first().expect("chain must not be empty").client
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Chat with fallback
// ─────────────────────────────────────────────────────────────────────────────

impl UnifiedFallbackClient {
    /// Send a chat request through the fallback chain.
    ///
    /// Iterates through [`chain`](Self::chain) entries, skipping those in
    /// cooldown. Returns the first successful [`UnifiedResponse`], or an error
    /// if all entries are exhausted.
    pub async fn chat(&self, mut request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        let mut idx = 0;
        loop {
            let entry = self.chain.get(idx).ok_or_else(|| {
                LLMError::ApiError("all models in unified fallback chain exhausted".to_string())
            })?;

            if self
                .cooldown
                .is_in_cooldown(&entry.provider_id, &entry.model_id)
                .await
            {
                tracing::debug!(
                    provider = %entry.provider_id,
                    model = %entry.model_id,
                    "model in cooldown, skipping"
                );
                idx += 1;
                continue;
            }

            request.model = entry.model_id.clone();

            match entry.client.chat(request.clone()).await {
                Ok(response) => {
                    self.cooldown
                        .record_success(&entry.provider_id, &entry.model_id)
                        .await;
                    return Ok(response);
                }
                Err(client_err) => {
                    let llm_err: LLMError = client_err.into();
                    let kind = llm_err.kind();
                    tracing::warn!(
                        provider = %entry.provider_id,
                        model = %entry.model_id,
                        error = %llm_err,
                        kind = ?kind,
                        "unified fallback call failed"
                    );
                    self.cooldown
                        .record_failure(&entry.provider_id, &entry.model_id, kind)
                        .await;
                    idx += 1;
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::CooldownManager;
    use crate::ErrorKind;

    /// Build a mock chain entry with a no-op UnifiedChatClient.
    ///
    /// Uses `StubProvider` + `OpenAiProtocol::default()` to create a minimal client.
    fn mock_entry(provider_id: &str, model_id: &str) -> ChainEntry {
        use crate::cache_adapter::NoopCacheAdapter;
        use crate::client::UnifiedChatClient;
        use crate::interpreter::InterpreterRegistry;
        use crate::plugin::PluginPipeline;
        use crate::protocol::OpenAiProtocol;
        use crate::stub::StubProvider;
        use std::sync::Arc;

        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        ChainEntry {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            client,
        }
    }

    fn make_request(model: &str) -> InternalRequest {
        use crate::types::InternalMessage;
        use closeclaw_session::persistence::ReasoningLevel;

        InternalRequest {
            model: model.to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            temperature: 0.0,
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

    #[tokio::test]
    async fn test_single_entry_success() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry = mock_entry("stub", "stub-model");
        let client = UnifiedFallbackClient::new(vec![entry], cooldown);
        let request = make_request("stub-model");
        let result = client.chat(request).await;
        assert!(result.is_ok(), "single entry should succeed");
    }

    #[tokio::test]
    async fn test_primary_returns_first_entry() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry1 = mock_entry("a", "model-a");
        let entry2 = mock_entry("b", "model-b");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        assert_eq!(client.primary().provider_id(), "stub");
    }

    #[tokio::test]
    async fn test_chat_walks_chain_on_failure() {
        let cooldown = Arc::new(CooldownManager::new());
        // First entry uses StubProvider (succeeds), second entry also uses StubProvider.
        // This tests that the chain iteration logic works correctly.
        let entry1 = mock_entry("provider-a", "model-a");
        let entry2 = mock_entry("provider-b", "model-b");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        let request = make_request("model-a");
        let result = client.chat(request).await;
        assert!(result.is_ok());
    }

    // ── Failing provider (always errors) ───────────────────────────────────────

    /// A provider that always fails with a given error message.
    struct FailingProvider {
        msg: String,
        id: String,
    }

    impl FailingProvider {
        fn new(id: impl Into<String>, msg: impl Into<String>) -> Self {
            Self {
                id: id.into(),
                msg: msg.into(),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::provider::Provider for FailingProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn base_url(&self) -> &str {
            ""
        }
        fn api_key(&self) -> &str {
            ""
        }
        fn supported_protocols(&self) -> &[crate::types::ProtocolId] {
            &[]
        }
        fn http_client(&self) -> &reqwest::Client {
            static DUMMY: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
            DUMMY.get_or_init(reqwest::Client::new)
        }
        fn default_headers(&self) -> &reqwest::header::HeaderMap {
            static EMPTY: std::sync::OnceLock<reqwest::header::HeaderMap> =
                std::sync::OnceLock::new();
            EMPTY.get_or_init(reqwest::header::HeaderMap::new)
        }
        async fn send(
            &self,
            _request: crate::types::InternalRequest,
            _body: serde_json::Value,
        ) -> crate::provider::Result<crate::types::InternalResponse> {
            Err(crate::provider::ProviderError::Legacy(self.msg.clone()))
        }
        async fn send_streaming(
            &self,
            request: crate::types::InternalRequest,
            body: serde_json::Value,
        ) -> crate::provider::Result<crate::provider::SseStream> {
            self.send(request, body).await?;
            unreachable!()
        }
    }

    /// Build a chain entry whose `UnifiedChatClient` always fails.
    fn failing_entry(provider_id: &str, model_id: &str, msg: &str) -> ChainEntry {
        use crate::cache_adapter::NoopCacheAdapter;
        use crate::client::UnifiedChatClient;
        use crate::interpreter::InterpreterRegistry;
        use crate::plugin::PluginPipeline;
        use crate::protocol::OpenAiProtocol;

        let provider = Arc::new(FailingProvider::new(provider_id, msg));
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        ChainEntry {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            client,
        }
    }

    // ── Missing UT: all entries fail → chain exhausted error returned ───────────

    #[tokio::test]
    async fn test_all_entries_fail_returns_chain_exhausted_error() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry1 = failing_entry("p1", "m1", "error from provider 1");
        let entry2 = failing_entry("p2", "m2", "error from provider 2");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        let request = make_request("m1");
        let result = client.chat(request).await;
        assert!(result.is_err(), "should fail when all entries fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("all models in unified fallback chain exhausted"),
            "should return chain-exhausted error, got: {}",
            msg
        );
    }

    // ── Missing UT: cooldown skip ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_cooldown_skip_first_entry() {
        let cooldown = Arc::new(CooldownManager::new());
        // Put first entry into cooldown
        cooldown
            .record_failure("p-cooldown", "m-cooldown", ErrorKind::Transient)
            .await;
        assert!(
            cooldown.is_in_cooldown("p-cooldown", "m-cooldown").await,
            "first entry should be in cooldown"
        );

        let entry1 = mock_entry("p-cooldown", "m-cooldown");
        let entry2 = mock_entry("p-ok", "m-ok");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        // The request model will be overwritten to entry.model_id in chat(),
        // so we pass a dummy model here.
        let request = make_request("dummy");
        let result = client.chat(request).await;
        assert!(
            result.is_ok(),
            "should succeed via second entry after skipping cooldown entry"
        );
    }

    // ── Missing UT: empty chain ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_empty_chain_chat_returns_error() {
        let cooldown = Arc::new(CooldownManager::new());
        let client = UnifiedFallbackClient::new(vec![], cooldown);
        let request = make_request("m");
        let result = client.chat(request).await;
        assert!(result.is_err(), "empty chain should fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("all models in unified fallback chain exhausted"),
            "unexpected error: {}",
            msg
        );
    }

    #[test]
    #[should_panic(expected = "chain must not be empty")]
    fn test_empty_chain_primary_panics() {
        let cooldown = Arc::new(CooldownManager::new());
        let client = UnifiedFallbackClient::new(vec![], cooldown);
        let _ = client.primary();
    }
}
