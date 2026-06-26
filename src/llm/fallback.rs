//! LLM Fallback Chain Client
//!
//! Wraps LLM calls with retry, cooldown tracking, and model-level fallback.

use crate::llm::provider::Provider;
use crate::llm::retry::{
    backoff_delay, CooldownManager, MAX_TRANSIENT_RETRIES, MAX_UNKNOWN_RETRIES,
    TRANSIENT_BASE_DELAY, TRANSIENT_MAX_DELAY,
};
use crate::llm::types::{InternalMessage, InternalRequest, InternalResponse};
use crate::llm::{ChatRequest, ChatResponse, ErrorKind, LLMError};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Default LLM call timeout (30s per attempt)
const DEFAULT_CALL_TIMEOUT_SECS: u64 = 30;

/// LLM fallback client that wraps a provider with retry + fallback chain
pub struct FallbackClient {
    registry: Arc<crate::llm::LLMRegistry>,
    pub(crate) fallback_chain: Vec<ModelEntry>,
    cooldown: Arc<CooldownManager>,
    pub(crate) call_timeout: Duration,
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

// --- Request/response conversion helpers ---

impl FallbackClient {
    /// Convert a [`ChatRequest`] into an [`InternalRequest`].
    fn chat_request_to_internal(request: &ChatRequest) -> InternalRequest {
        InternalRequest {
            model: request.model.clone(),
            messages: request
                .messages
                .iter()
                .map(|m| InternalMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                })
                .collect(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: crate::session::persistence::ReasoningLevel::default(),
            turn_count: None,
        }
    }

    /// Convert an [`InternalResponse`] into a [`ChatResponse`].
    fn internal_to_chat_response(response: InternalResponse) -> ChatResponse {
        // Extract text content from content blocks
        let content: String = response
            .content_blocks
            .iter()
            .filter_map(|block| match block {
                crate::llm::types::RawContentBlock::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Build usage (take from first block's usage or default)
        let usage = crate::llm::Usage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens.unwrap_or(0),
        };

        ChatResponse {
            content,
            model: String::new(), // Not available from InternalResponse
            usage,
        }
    }

    /// Call a provider via the [`Provider`] trait, converting request/response types.
    async fn call_provider(
        &self,
        provider: &Arc<dyn Provider>,
        request: ChatRequest,
    ) -> Result<ChatResponse, LLMError> {
        let internal_req = Self::chat_request_to_internal(&request);
        let body = serde_json::to_value(&internal_req)
            .map_err(|e| LLMError::InvalidRequest(e.to_string()))?;
        let internal_resp = provider
            .send(internal_req, body)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;
        Ok(Self::internal_to_chat_response(internal_resp))
    }

    /// Call a provider via the [`Provider`] trait, returning a [`UnifiedResponse`].
    async fn call_provider_unified(
        &self,
        provider: &Arc<dyn Provider>,
        request: ChatRequest,
    ) -> Result<crate::llm::types::UnifiedResponse, LLMError> {
        let internal_req = Self::chat_request_to_internal(&request);
        let body = serde_json::to_value(&internal_req)
            .map_err(|e| LLMError::InvalidRequest(e.to_string()))?;
        let internal_resp = provider
            .send(internal_req, body)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;
        Ok(crate::llm::types::UnifiedResponse::from(internal_resp))
    }
}

// --- Chat with fallback ---

impl FallbackClient {
    /// Make a chat request with automatic retry and fallback.
    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        self.try_fallback_chain(request).await
    }

    /// Make a chat request returning structured content blocks.
    pub async fn chat_unified(
        &self,
        request: ChatRequest,
    ) -> Result<crate::llm::types::UnifiedResponse, LLMError> {
        self.try_fallback_chain_unified(request).await
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
            match self
                .call_provider_with_retry(&provider, request.clone())
                .await
            {
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

// --- Unified chat with fallback ---

impl FallbackClient {
    /// Walk the fallback chain using chat_unified() until one model succeeds.
    async fn try_fallback_chain_unified(
        &self,
        mut request: ChatRequest,
    ) -> Result<crate::llm::types::UnifiedResponse, LLMError> {
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
            match self.call_provider_unified(&provider, request.clone()).await {
                Ok(response) => {
                    self.cooldown
                        .record_success(&entry.provider, &entry.model)
                        .await;
                    return Ok(response);
                }
                Err(err) => {
                    let kind = err.kind();
                    tracing::warn!(
                        provider = %entry.provider, model = %entry.model, error = %err, kind = ?kind, "LLM unified call failed"
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
    async fn call_provider_with_retry(
        &self,
        provider: &Arc<dyn Provider>,
        request: ChatRequest,
    ) -> Result<ChatResponse, LLMError> {
        let max_retries = MAX_TRANSIENT_RETRIES;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let result = tokio::time::timeout(
                self.call_timeout,
                self.call_provider(provider, request.clone()),
            )
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
