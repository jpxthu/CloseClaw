//! LLM Fallback Chain Client
//!
//! Wraps LLM calls with retry, cooldown tracking, and model-level fallback.

use crate::llm::retry::{
    backoff_delay, CooldownManager, MAX_TRANSIENT_RETRIES, MAX_UNKNOWN_RETRIES,
    TRANSIENT_BASE_DELAY, TRANSIENT_MAX_DELAY,
};
use crate::llm::{ChatRequest, ChatResponse, ErrorKind, LLMError, LLMProvider};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Default LLM call timeout (30s per attempt)
const DEFAULT_CALL_TIMEOUT_SECS: u64 = 30;

/// LLM fallback client that wraps a provider with retry + fallback chain
pub struct FallbackClient {
    registry: Arc<crate::llm::LLMRegistry>,
    fallback_chain: Vec<ModelEntry>,
    cooldown: Arc<CooldownManager>,
    call_timeout: Duration,
}

/// A model entry with provider name and model name
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub provider: String,
    pub model: String,
}

// --- Constructors ---

impl FallbackClient {
    /// Create a new FallbackClient with the given registry and fallback chain.
    pub fn new(registry: Arc<crate::llm::LLMRegistry>, fallback_chain: Vec<ModelEntry>) -> Self {
        let cooldown = Arc::new(CooldownManager::new());
        cooldown.load_sync();
        Self {
            registry,
            fallback_chain,
            cooldown,
            call_timeout: Duration::from_secs(DEFAULT_CALL_TIMEOUT_SECS),
        }
    }

    /// Async constructor: creates the client and loads persisted cooldowns.
    pub async fn new_async(
        registry: Arc<crate::llm::LLMRegistry>,
        fallback_chain: Vec<ModelEntry>,
    ) -> Self {
        let cooldown = Arc::new(CooldownManager::new());
        cooldown.load().await;
        Self {
            registry,
            fallback_chain,
            cooldown,
            call_timeout: Duration::from_secs(DEFAULT_CALL_TIMEOUT_SECS),
        }
    }

    /// Create from config-style strings like "minimax/MiniMax-M2.7"
    pub fn from_strings(registry: Arc<crate::llm::LLMRegistry>, chain: Vec<String>) -> Self {
        let fallback_chain: Vec<ModelEntry> = chain
            .into_iter()
            .filter_map(|s| {
                let (provider, model) = s.split_once('/')?;
                Some(ModelEntry {
                    provider: provider.to_string(),
                    model: model.to_string(),
                })
            })
            .collect();
        Self::new(registry, fallback_chain)
    }

    /// Set call timeout
    #[allow(dead_code)]
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.call_timeout = Duration::from_secs(secs);
        self
    }
}

// --- Chat with fallback ---

impl FallbackClient {
    /// Make a chat request with automatic retry and fallback.
    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        self.try_fallback_chain(request).await
    }

    /// Walk the fallback chain until one model succeeds or all are exhausted.
    async fn try_fallback_chain(&self, mut request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let mut model_idx = 0;
        loop {
            let entry = self.fallback_chain.get(model_idx).ok_or_else(|| {
                LLMError::ApiError("all models in fallback chain exhausted".to_string())
            })?;
            if self
                .cooldown
                .is_in_cooldown(&entry.provider, &entry.model)
                .await
            {
                tracing::debug!(provider = %entry.provider, model = %entry.model, "model in cooldown, skipping");
                model_idx += 1;
                continue;
            }
            let provider = match self.registry.get(&entry.provider).await {
                Some(p) => p,
                None => {
                    tracing::warn!(provider = %entry.provider, "provider not found, trying next");
                    model_idx += 1;
                    continue;
                }
            };
            request.model = entry.model.clone();
            match self.chat_with_retry(&provider, request.clone()).await {
                Ok(response) => {
                    self.cooldown
                        .record_success(&entry.provider, &entry.model)
                        .await;
                    return Ok(response);
                }
                Err(err) => {
                    let kind = err.kind();
                    tracing::warn!(
                        provider = %entry.provider, model = %entry.model, error = %err, kind = ?kind, "LLM call failed"
                    );
                    self.cooldown
                        .record_failure(&entry.provider, &entry.model, kind)
                        .await;
                    model_idx += 1;
                }
            }
        }
    }
}

// --- Retry logic ---

impl FallbackClient {
    /// Call with retry (exponential backoff) for transient errors
    async fn chat_with_retry(
        &self,
        provider: &Arc<dyn LLMProvider>,
        request: ChatRequest,
    ) -> Result<ChatResponse, LLMError> {
        let max_retries = MAX_TRANSIENT_RETRIES;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let result = tokio::time::timeout(self.call_timeout, provider.chat(request.clone()))
                .await
                .map_err(|_| LLMError::NetworkError("call timed out".to_string()))
                .and_then(|r| r);
            match result {
                Ok(response) => return Ok(response),
                Err(err) => {
                    let kind = err.kind();
                    if kind == ErrorKind::Transient || kind == ErrorKind::Unknown {
                        if attempt >= max_retries
                            || (kind == ErrorKind::Unknown && attempt >= MAX_UNKNOWN_RETRIES)
                        {
                            return Err(err);
                        }
                        let delay =
                            backoff_delay(attempt, TRANSIENT_BASE_DELAY, TRANSIENT_MAX_DELAY);
                        tracing::debug!(attempt = %attempt, delay_secs = %delay.as_secs(), "retrying after backoff");
                        sleep(delay).await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider};

    #[test]
    fn test_model_entry_parse() {
        let entry = ModelEntry {
            provider: "minimax".to_string(),
            model: "MiniMax-M2.7".to_string(),
        };
        assert_eq!(entry.provider, "minimax");
        assert_eq!(entry.model, "MiniMax-M2.7");
    }

    #[tokio::test]
    async fn test_fallback_client_requires_registry() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        let client = FallbackClient::from_strings(registry, vec![]);
        let req = ChatRequest {
            model: "MiniMax-M2.7".to_string(),
            messages: vec![],
            temperature: 0.7,
            max_tokens: Some(100),
        };
        let err = client.chat(req).await.unwrap_err();
        assert!(err.to_string().contains("exhausted"));
    }

    // --- Mock provider for fallback chain tests ---

    struct MockProvider {
        name: String,
        response_fn: Box<dyn Fn() -> Result<ChatResponse, LLMError> + Send + Sync>,
    }

    impl MockProvider {
        fn new(name: &str, response: Result<ChatResponse, LLMError>) -> Self {
            let r = Arc::new(response);
            Self {
                name: name.to_string(),
                response_fn: Box::new(move || match Arc::as_ref(&r) {
                    Ok(v) => Ok(v.clone()),
                    Err(e) => {
                        // Reconstruct error since LLMError isn't Clone
                        match e {
                            LLMError::AuthFailed(msg) => Err(LLMError::AuthFailed(msg.clone())),
                            LLMError::RateLimitExceeded => Err(LLMError::RateLimitExceeded),
                            LLMError::ModelNotFound(msg) => Err(LLMError::ModelNotFound(msg.clone())),
                            LLMError::InvalidRequest(msg) => Err(LLMError::InvalidRequest(msg.clone())),
                            LLMError::ApiError(msg) => Err(LLMError::ApiError(msg.clone())),
                            LLMError::NetworkError(msg) => Err(LLMError::NetworkError(msg.clone())),
                        }
                    }
                }),
            }
        }
    }

    #[async_trait::async_trait]
    impl LLMProvider for MockProvider {
        fn name(&self) -> &str { &self.name }
        fn models(&self) -> Vec<&str> { vec!["test-model"] }
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LLMError> {
            (self.response_fn)()
        }
    }

    fn ok_response() -> ChatResponse {
        ChatResponse {
            model: "test-model".to_string(),
            content: "hello".to_string(),
            usage: crate::llm::Usage { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
        }
    }

    #[tokio::test]
    async fn test_fallback_client_succeeds_on_first_model() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        registry.register("prov".to_string(), Arc::new(MockProvider::new("prov", Ok(ok_response())))).await;

        let client = FallbackClient::from_strings(registry, vec!["prov/test-model".to_string()]);
        let req = ChatRequest {
            model: "test-model".to_string(),
            messages: vec![],
            temperature: 0.7,
            max_tokens: Some(100),
        };
        let result = client.chat(req).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content, "hello");
    }

    #[tokio::test]
    async fn test_fallback_client_falls_through_on_auth_error() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        // First provider fails with auth error
        registry.register("fail".to_string(), Arc::new(MockProvider::new("fail", Err(LLMError::AuthFailed("bad key".to_string()))))).await;
        // Second provider succeeds
        registry.register("ok".to_string(), Arc::new(MockProvider::new("ok", Ok(ok_response())))).await;

        let client = FallbackClient::new(registry, vec![
            ModelEntry { provider: "fail".to_string(), model: "m1".to_string() },
            ModelEntry { provider: "ok".to_string(), model: "m2".to_string() },
        ]);
        let req = ChatRequest {
            model: "m1".to_string(),
            messages: vec![],
            temperature: 0.7,
            max_tokens: Some(100),
        };
        let result = client.chat(req).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_fallback_client_skips_missing_provider() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        registry.register("ok".to_string(), Arc::new(MockProvider::new("ok", Ok(ok_response())))).await;

        let client = FallbackClient::new(registry, vec![
            ModelEntry { provider: "missing".to_string(), model: "m1".to_string() },
            ModelEntry { provider: "ok".to_string(), model: "m2".to_string() },
        ]);
        let req = ChatRequest {
            model: "m1".to_string(),
            messages: vec![],
            temperature: 0.7,
            max_tokens: Some(100),
        };
        let result = client.chat(req).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_fallback_client_all_exhausted() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        registry.register("fail".to_string(), Arc::new(MockProvider::new("fail", Err(LLMError::InvalidRequest("bad".to_string()))))).await;

        let client = FallbackClient::from_strings(registry, vec!["fail/test-model".to_string()]);
        let req = ChatRequest {
            model: "test-model".to_string(),
            messages: vec![],
            temperature: 0.7,
            max_tokens: Some(100),
        };
        let err = client.chat(req).await.unwrap_err();
        assert!(err.to_string().contains("exhausted"));
    }

    #[test]
    fn test_from_strings_parses_provider_model() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        let client = FallbackClient::from_strings(
            registry,
            vec!["prov-a/model-1".to_string(), "prov-b/model-2".to_string()],
        );
        assert_eq!(client.fallback_chain.len(), 2);
        assert_eq!(client.fallback_chain[0].provider, "prov-a");
        assert_eq!(client.fallback_chain[1].model, "model-2");
    }

    #[test]
    fn test_from_strings_skips_invalid() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        let client = FallbackClient::from_strings(
            registry,
            vec!["valid/model".to_string(), "no-slash".to_string()],
        );
        assert_eq!(client.fallback_chain.len(), 1);
    }

    #[test]
    fn test_with_timeout() {
        let registry = Arc::new(crate::llm::LLMRegistry::new());
        let client = FallbackClient::new(registry, vec![]).with_timeout(60);
        assert_eq!(client.call_timeout, Duration::from_secs(60));
    }
}
