//! Model Discovery Service
//!
//! Three-layer model discovery: cache → API (with retry) → knowledge base fallback.

use std::future::Future;
use std::time::Duration;

use super::model_cache::ModelCache;
use super::model_info::{DiscoveryResult, DiscoverySource, ModelInfo};
use super::{ErrorKind, ProviderModelKnowledge};

/// Maximum number of fetch retries for transient errors.
const FETCH_MAX_RETRIES: u32 = 3;
/// Per-attempt API timeout.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Model discovery service combining local cache, API fetch, and knowledge base.
pub struct ModelDiscovery {
    pub(crate) cache: ModelCache,
    pub(crate) knowledge: ProviderModelKnowledge,
}

impl ModelDiscovery {
    /// Create a new `ModelDiscovery` with default cache and knowledge base.
    pub fn new() -> Self {
        Self {
            cache: ModelCache::new(),
            knowledge: ProviderModelKnowledge::new(),
        }
    }

    /// Discover available models for a provider.
    ///
    /// 1. Check local cache — return immediately if hit and not expired.
    /// 2. Cache miss: call `fetch` closure with 10s timeout + 3-retry backoff.
    /// 3. On fetch success: write to cache and return.
    /// 4. On fetch failure: fall back to knowledge base.
    pub async fn discover<F, Fut>(
        &self,
        provider_id: &str,
        credential: &str,
        fetch: F,
    ) -> DiscoveryResult
    where
        F: Fn(&str) -> Fut,
        Fut: Future<Output = Result<Vec<ModelInfo>, crate::llm::LLMError>>,
    {
        // Layer 1: cache
        if let Some(models) = self.cache.get(provider_id, credential) {
            return DiscoveryResult {
                models,
                source: DiscoverySource::Cache,
            };
        }

        // Layer 2: API fetch with retry
        for attempt in 1..=FETCH_MAX_RETRIES {
            let result = tokio::time::timeout(FETCH_TIMEOUT, fetch(credential)).await;

            match result {
                Ok(Ok(mut models)) => {
                    for model in &mut models {
                        if let Some(params) = self.knowledge.find(provider_id, &model.id) {
                            model.reasoning = params.reasoning;
                            if params.context_window > model.context_window {
                                model.context_window = params.context_window;
                            }
                            if params.max_tokens > model.max_tokens {
                                model.max_tokens = params.max_tokens;
                            }
                            // Fill default_temperature when API provides None
                            if model.default_temperature.is_none() {
                                model.default_temperature = Some(params.default_temperature);
                            }
                            // Fill input_types when API returns empty
                            if model.input_types.is_empty() {
                                model.input_types = params.input_types;
                            }
                        }
                    }
                    self.cache.set(provider_id, credential, models.clone());
                    return DiscoveryResult {
                        models,
                        source: DiscoverySource::Api,
                    };
                }
                Ok(Err(err)) => {
                    let kind = err.kind();
                    if !matches!(kind, ErrorKind::Transient | ErrorKind::Unknown) {
                        // Auth / Billing / InvalidRequest → immediate fallback
                        break;
                    }
                    if attempt < FETCH_MAX_RETRIES {
                        let delay = super::retry::backoff_delay(
                            attempt,
                            Duration::from_secs(1),
                            Duration::from_secs(10),
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                }
                Err(_) => {
                    // Timeout → transient, retry
                    if attempt < FETCH_MAX_RETRIES {
                        let delay = super::retry::backoff_delay(
                            attempt,
                            Duration::from_secs(1),
                            Duration::from_secs(10),
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                }
            }
        }

        // Layer 3: knowledge base fallback
        self.knowledge_fallback(provider_id)
    }

    /// Return all known models for a provider from the embedded knowledge base.
    pub fn knowledge_fallback(&self, provider_id: &str) -> DiscoveryResult {
        let model_ids = self.knowledge.all_models(provider_id);
        let models = model_ids
            .into_iter()
            .map(|id| {
                let params = self.knowledge.find(provider_id, id).unwrap();
                ModelInfo {
                    id: id.to_string(),
                    name: id.to_string(),
                    context_window: params.context_window,
                    max_tokens: params.max_tokens,
                    default_temperature: Some(params.default_temperature),
                    reasoning: params.reasoning,
                    input_types: params.input_types,
                }
            })
            .collect();
        DiscoveryResult {
            models,
            source: DiscoverySource::KnowledgeFallback,
        }
    }
}

impl Default for ModelDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::model_cache::{CacheEntry, CacheKey};
    use crate::llm::LLMError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Helper: create a ModelDiscovery with an isolated temp-dir cache.
    fn make_discovery(dir: &tempfile::TempDir) -> ModelDiscovery {
        let path = dir.path().join("cache.json");
        std::env::set_var("MODEL_CACHE_FILE", &path);
        let cache = ModelCache::new();
        ModelDiscovery {
            cache,
            knowledge: ProviderModelKnowledge::new(),
        }
    }

    fn test_models() -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "test-model-1".into(),
            name: "Test Model 1".into(),
            context_window: 4096,
            max_tokens: 1024,
            default_temperature: Some(0.7),
            reasoning: false,
            input_types: vec![],
        }]
    }

    #[tokio::test]
    async fn test_cache_hit_returns_cached_and_no_fetch_call() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = make_discovery(&dir);

        // Pre-populate cache
        discovery
            .cache
            .set("test-provider", "mytoken", test_models());

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fc = fetch_count.clone();
        let result = discovery
            .discover("test-provider", "mytoken", move |_| {
                let fc = fc.clone();
                async move {
                    fc.fetch_add(1, Ordering::SeqCst);
                    Ok(test_models())
                }
            })
            .await;

        assert_eq!(result.models().len(), 1);
        assert_eq!(result.models()[0].id, "test-model-1");
        // fetch closure must NOT have been called
        assert_eq!(fetch_count.load(Ordering::SeqCst), 0);

        std::env::remove_var("MODEL_CACHE_FILE");
    }

    #[tokio::test]
    async fn test_cache_miss_fetch_success_writes_cache() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = make_discovery(&dir);

        let result = discovery
            .discover("test-provider", "mytoken", |_| async { Ok(test_models()) })
            .await;

        assert_eq!(result.models().len(), 1);
        assert_eq!(result.models()[0].id, "test-model-1");

        // Cache should now be populated — second call should not invoke fetch
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fc = fetch_count.clone();
        let result2 = discovery
            .discover("test-provider", "mytoken", move |_| {
                let fc = fc.clone();
                async move {
                    fc.fetch_add(1, Ordering::SeqCst);
                    Ok(test_models())
                }
            })
            .await;

        assert_eq!(result2.models().len(), 1);
        assert_eq!(fetch_count.load(Ordering::SeqCst), 0);

        std::env::remove_var("MODEL_CACHE_FILE");
    }

    #[tokio::test]
    async fn test_cache_miss_fetch_failure_falls_back_to_knowledge() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = make_discovery(&dir);

        // Use a provider that exists in knowledge base — "minimax"
        let result = discovery
            .discover("minimax", "mytoken", |_| async {
                Err(LLMError::AuthFailed("bad key".into()))
            })
            .await;

        // Should fall back to knowledge base — minimax has models
        assert!(
            !result.models().is_empty(),
            "knowledge fallback should return models"
        );

        std::env::remove_var("MODEL_CACHE_FILE");
    }

    #[tokio::test]
    async fn test_expired_cache_triggers_refetch() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = make_discovery(&dir);

        // Manually write an expired cache entry
        let key = CacheKey::compute("test-provider", &CacheKey::token_prefix("mytoken"));
        let expired_entry = CacheEntry {
            fetched_at: chrono::Utc::now().timestamp() - 999_999,
            ttl_secs: 3600,
            models: vec![ModelInfo {
                id: "old-model".into(),
                name: "Old".into(),
                context_window: 0,
                max_tokens: 0,
                default_temperature: None,
                reasoning: false,
                input_types: vec![],
            }],
        };
        let mut map = std::collections::HashMap::new();
        map.insert(key, expired_entry);
        let path = dir.path().join("cache.json");
        std::fs::write(&path, serde_json::to_string_pretty(&map).unwrap()).unwrap();

        let result = discovery
            .discover("test-provider", "mytoken", |_| async { Ok(test_models()) })
            .await;

        // Should have re-fetched (not returned old-model)
        assert_eq!(result.models().len(), 1);
        assert_eq!(result.models()[0].id, "test-model-1");

        std::env::remove_var("MODEL_CACHE_FILE");
    }

    #[test]
    fn test_knowledge_fallback_returns_known_models() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MODEL_CACHE_FILE", dir.path().join("cache.json"));
        let discovery = ModelDiscovery::new();
        std::env::remove_var("MODEL_CACHE_FILE");

        let result = discovery.knowledge_fallback("minimax");
        assert!(
            !result.models().is_empty(),
            "minimax should have known models"
        );
    }

    // ── Knowledge base filling tests (Step 1.3) ──────────────────────

    fn make_test_discovery(dir: &tempfile::TempDir) -> ModelDiscovery {
        ModelDiscovery {
            cache: ModelCache::with_path(dir.path().join("cache.json")),
            knowledge: ProviderModelKnowledge::new(),
        }
    }

    #[tokio::test]
    async fn test_discover_success_path_knowledge_fills() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = make_test_discovery(&dir);

        // API returns MiniMax-M2.7 with context_window=1000 (below knowledge base 32768)
        let api_models = vec![ModelInfo {
            id: "MiniMax-M2.7".into(),
            name: "MiniMax M2.7".into(),
            context_window: 1000,
            max_tokens: 512,
            default_temperature: Some(0.3),
            reasoning: false,
            input_types: vec![],
        }];

        let result = discovery
            .discover("minimax", "key", |_| {
                let value = api_models.clone();
                async move { Ok(value) }
            })
            .await;

        assert_eq!(result.models().len(), 1);
        let m = &result.models()[0];
        // Knowledge base fills reasoning: true for M2.7
        assert!(
            m.reasoning,
            "M2.7 should have reasoning=true from knowledge base"
        );
        // Knowledge base context_window (204800) > API (1000), so it overrides
        assert_eq!(m.context_window, 204_800);
        // Knowledge base max_tokens (131072) > API (512), so it overrides
        assert_eq!(m.max_tokens, 131_072);
    }

    #[tokio::test]
    async fn test_discover_knowledge_miss_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let discovery = make_test_discovery(&dir);

        // API returns an unknown model not in knowledge base
        let api_models = vec![ModelInfo {
            id: "some-new-future-model".into(),
            name: "Future Model".into(),
            context_window: 16384,
            max_tokens: 4096,
            default_temperature: Some(0.5),
            reasoning: false,
            input_types: vec![],
        }];

        let result = discovery
            .discover("minimax", "key", |_| {
                let value = api_models.clone();
                async move { Ok(value) }
            })
            .await;

        assert_eq!(result.models().len(), 1);
        let m = &result.models()[0];
        // Unknown model: no knowledge base fill, API values preserved
        assert_eq!(m.id, "some-new-future-model");
        assert_eq!(m.context_window, 16384);
        assert_eq!(m.max_tokens, 4096);
        assert!(!m.reasoning);
    }
}
