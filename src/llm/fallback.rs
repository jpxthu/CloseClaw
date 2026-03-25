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
    /// The underlying LLM registry
    registry: Arc<crate::llm::LLMRegistry>,
    /// Fallback chain: first is primary, rest are fallbacks
    fallback_chain: Vec<ModelEntry>,
    /// Cooldown manager
    cooldown: Arc<CooldownManager>,
    /// Timeout per call
    call_timeout: Duration,
}

/// A model entry with provider name and model name
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub provider: String,
    pub model: String,
}

impl FallbackClient {
    /// Create a new FallbackClient with the given registry and fallback chain.
    ///
    /// `fallback_chain` is a list like `["minimax/MiniMax-M2.7", "dashscope/qwen3-max"]`
    pub fn new(registry: Arc<crate::llm::LLMRegistry>, fallback_chain: Vec<ModelEntry>) -> Self {
        let cooldown = Arc::new(CooldownManager::new());
        // Load persisted cooldowns from disk (no-op if no runtime is running yet,
        // which is the startup case; skipped when called from within a runtime).
        cooldown.load_sync();
        Self {
            registry,
            fallback_chain,
            cooldown,
            call_timeout: Duration::from_secs(DEFAULT_CALL_TIMEOUT_SECS),
        }
    }

    /// Async constructor: creates the client and loads persisted cooldowns.
    /// Prefer this in async contexts where block_on is unavailable.
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
                let parts: Vec<&str> = s.splitn(2, '/').collect();
                if parts.len() == 2 {
                    Some(ModelEntry {
                        provider: parts[0].to_string(),
                        model: parts[1].to_string(),
                    })
                } else {
                    None
                }
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

    /// Make a chat request with automatic retry and fallback.
    ///
    /// Implements two-stage failover:
    /// 1. Retry current model with exponential backoff (up to MAX_TRANSIENT_RETRIES)
    /// 2. If retries exhausted, switch to next model in fallback chain
    pub async fn chat(&self, mut request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let mut model_idx = 0;

        loop {
            let entry = self.fallback_chain.get(model_idx).ok_or_else(|| {
                LLMError::ApiError("all models in fallback chain exhausted".to_string())
            })?;

            // Check cooldown
            if self
                .cooldown
                .is_in_cooldown(&entry.provider, &entry.model)
                .await
            {
                // Model is in cooldown, try next in chain
                tracing::debug!(provider = %entry.provider, model = %entry.model, "model in cooldown, skipping");
                model_idx += 1;
                continue;
            }

            // Get provider
            let provider = match self.registry.get(&entry.provider).await {
                Some(p) => p,
                None => {
                    tracing::warn!(provider = %entry.provider, "provider not found, trying next");
                    model_idx += 1;
                    continue;
                }
            };

            // Update request model to use the current model entry
            request.model = entry.model.clone();

            // Try the call with timeout + retry
            match self.chat_with_retry(&provider, request.clone()).await {
                Ok(response) => {
                    // Success — record it to clear cooldown
                    self.cooldown
                        .record_success(&entry.provider, &entry.model)
                        .await;
                    return Ok(response);
                }
                Err(err) => {
                    let kind = err.kind();
                    tracing::warn!(
                        provider = %entry.provider,
                        model = %entry.model,
                        error = %err,
                        kind = ?kind,
                        "LLM call failed"
                    );

                    match kind {
                        ErrorKind::InvalidRequest => {
                            // Don't retry invalid request, switch model immediately
                            self.cooldown
                                .record_failure(&entry.provider, &entry.model, kind)
                                .await;
                            model_idx += 1;
                        }
                        ErrorKind::Auth => {
                            // Auth errors: long cooldown + try next model
                            self.cooldown
                                .record_failure(&entry.provider, &entry.model, kind)
                                .await;
                            model_idx += 1;
                        }
                        ErrorKind::Transient | ErrorKind::Unknown => {
                            // Record failure (for unknown, limited retries)
                            self.cooldown
                                .record_failure(&entry.provider, &entry.model, kind)
                                .await;
                            // Stay on same model for retry (exhausted via MAX_RETRIES check in chat_with_retry)
                            // If chat_with_retry returned error, it means retries exhausted
                            model_idx += 1;
                        }
                        ErrorKind::Billing => {
                            // Billing: long cooldown + try next model
                            self.cooldown
                                .record_failure(&entry.provider, &entry.model, kind)
                                .await;
                            model_idx += 1;
                        }
                    }
                }
            }
        }
    }

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
                        // Check retry budget
                        if attempt >= max_retries
                            || (kind == ErrorKind::Unknown && attempt >= MAX_UNKNOWN_RETRIES)
                        {
                            return Err(err);
                        }

                        // Calculate backoff
                        let delay =
                            backoff_delay(attempt, TRANSIENT_BASE_DELAY, TRANSIENT_MAX_DELAY);
                        tracing::debug!(attempt = %attempt, delay_secs = %delay.as_secs(), "retrying after backoff");
                        sleep(delay).await;
                        continue;
                    }

                    // Non-retryable error, propagate
                    return Err(err);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
