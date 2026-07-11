//! LLM Fallback Chain Client
//!
//! Wraps LLM calls with retry, cooldown tracking, and model-level fallback.
//!
//! [`FallbackClient`] supports both non-streaming ([`chat`](Self::chat),
//! [`chat_unified`](Self::chat_unified)) and streaming
//! ([`chat_streaming`](Self::chat_streaming)) requests.  Streaming walks the
//! provider chain calling [`Provider::send_streaming`] on each entry; when no
//! streaming provider is available it degrades to a non-streaming call and
//! wraps the response as a character-by-character stream.

use crate::protocol::{ChatProtocol, IncomingSseStream, OutgoingEventStream};
use crate::provider::Provider;
use crate::retry::{
    backoff_delay, CooldownManager, MAX_TRANSIENT_RETRIES, MAX_UNKNOWN_RETRIES,
    TRANSIENT_BASE_DELAY, TRANSIENT_MAX_DELAY,
};
use crate::stream_utils::ReceiverStream;
use crate::types::{InternalMessage, InternalRequest, InternalResponse};
use crate::{ChatRequest, ChatResponse, ErrorKind, LLMError};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Default LLM call timeout (30s per attempt)
const DEFAULT_CALL_TIMEOUT_SECS: u64 = 30;

/// LLM fallback client that wraps a provider with retry + fallback chain
pub struct FallbackClient {
    registry: Arc<crate::LLMRegistry>,
    pub(crate) fallback_chain: Vec<ModelEntry>,
    cooldown: Arc<CooldownManager>,
    pub(crate) call_timeout: Duration,
    /// Protocol used to parse SSE streams into [`StreamEvent`] values.
    protocol: Arc<dyn ChatProtocol>,
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
    pub fn new(registry: Arc<crate::LLMRegistry>, fallback_chain: Vec<ModelEntry>) -> Self {
        Self::new_with_protocol(
            registry,
            fallback_chain,
            Arc::new(crate::protocol::OpenAiProtocol::default()),
        )
    }

    /// Create a new FallbackClient with an explicit [`ChatProtocol`].
    pub fn new_with_protocol(
        registry: Arc<crate::LLMRegistry>,
        fallback_chain: Vec<ModelEntry>,
        protocol: Arc<dyn ChatProtocol>,
    ) -> Self {
        let cooldown = Arc::new(CooldownManager::new());
        cooldown.load_sync();
        Self {
            registry,
            fallback_chain,
            cooldown,
            call_timeout: Duration::from_secs(DEFAULT_CALL_TIMEOUT_SECS),
            protocol,
        }
    }

    /// Async constructor: creates the client and loads persisted cooldowns.
    pub async fn new_async(
        registry: Arc<crate::LLMRegistry>,
        fallback_chain: Vec<ModelEntry>,
    ) -> Self {
        Self::new_async_with_protocol(
            registry,
            fallback_chain,
            Arc::new(crate::protocol::OpenAiProtocol::default()),
        )
        .await
    }

    /// Async constructor with an explicit [`ChatProtocol`].
    pub async fn new_async_with_protocol(
        registry: Arc<crate::LLMRegistry>,
        fallback_chain: Vec<ModelEntry>,
        protocol: Arc<dyn ChatProtocol>,
    ) -> Self {
        let cooldown = Arc::new(CooldownManager::new());
        cooldown.load().await;
        Self {
            registry,
            fallback_chain,
            cooldown,
            call_timeout: Duration::from_secs(DEFAULT_CALL_TIMEOUT_SECS),
            protocol,
        }
    }

    /// Create from config-style strings like "minimax/MiniMax-M2.7"
    pub fn from_strings(registry: Arc<crate::LLMRegistry>, chain: Vec<String>) -> Self {
        Self::from_strings_with_protocol(
            registry,
            chain,
            Arc::new(crate::protocol::OpenAiProtocol::default()),
        )
    }

    /// Create from config-style strings with an explicit [`ChatProtocol`].
    pub fn from_strings_with_protocol(
        registry: Arc<crate::LLMRegistry>,
        chain: Vec<String>,
        protocol: Arc<dyn ChatProtocol>,
    ) -> Self {
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
        Self::new_with_protocol(registry, fallback_chain, protocol)
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
                    ..Default::default()
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
            reasoning_level: closeclaw_session::persistence::ReasoningLevel::default(),
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
                crate::types::RawContentBlock::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Build usage (take from first block's usage or default)
        let usage = crate::Usage {
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
    ) -> Result<crate::types::UnifiedResponse, LLMError> {
        let internal_req = Self::chat_request_to_internal(&request);
        let body = serde_json::to_value(&internal_req)
            .map_err(|e| LLMError::InvalidRequest(e.to_string()))?;
        let internal_resp = provider
            .send(internal_req, body)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;
        Ok(crate::types::UnifiedResponse::from(internal_resp))
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
    ) -> Result<crate::types::UnifiedResponse, LLMError> {
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
    ) -> Result<crate::types::UnifiedResponse, LLMError> {
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

// --- Streaming chat with fallback ---

impl FallbackClient {
    /// Send a streaming chat request through the fallback chain.
    ///
    /// Walks the chain trying [`Provider::send_streaming`] on each entry,
    /// skipping cooldown entries.  On success the raw SSE stream is parsed
    /// into [`StreamEvent`] values via the configured [`ChatProtocol`].
    /// On failure the cooldown is recorded and the next entry is tried.
    ///
    /// If every entry's streaming call fails, degrades to a non-streaming
    /// [`call_provider_unified`] and wraps the complete response as a
    /// character-by-character stream.
    pub async fn chat_streaming(
        &self,
        mut request: InternalRequest,
    ) -> Result<OutgoingEventStream, LLMError> {
        let mut idx = 0;
        loop {
            match self.fallback_chain.get(idx) {
                None => {
                    // All streaming entries exhausted — degrade to non-streaming.
                    return self.degraded_stream(request).await;
                }
                Some(entry) => match self.try_streaming_entry(entry, &mut request).await {
                    Ok(stream) => return Ok(stream),
                    Err(()) => {
                        idx += 1;
                    }
                },
            }
        }
    }
}

impl FallbackClient {
    /// Try streaming with a single provider entry.
    ///
    /// Returns `Ok(stream)` on success, or `Err(())` if the entry should be
    /// skipped (cooldown, missing provider, call failure).
    async fn try_streaming_entry(
        &self,
        entry: &ModelEntry,
        request: &mut InternalRequest,
    ) -> Result<OutgoingEventStream, ()> {
        if self
            .cooldown
            .is_in_cooldown(&entry.provider, &entry.model)
            .await
        {
            tracing::debug!(
                provider = %entry.provider,
                model = %entry.model,
                "model in cooldown, skipping"
            );
            return Err(());
        }

        let provider = match self.registry.get(&entry.provider).await {
            Some(p) => p,
            None => {
                tracing::warn!(
                    provider = %entry.provider,
                    "provider not found, trying next"
                );
                return Err(());
            }
        };

        request.model = entry.model.clone();
        let body = serde_json::to_value(&request)
            .map_err(|e| LLMError::InvalidRequest(e.to_string()))
            .map_err(|_| ())?;

        let result = tokio::time::timeout(
            self.call_timeout,
            provider.send_streaming(request.clone(), body),
        )
        .await;

        self.handle_streaming_result(entry, result).await
    }

    /// Handle the outcome of a streaming call (success, provider error, timeout).
    async fn handle_streaming_result(
        &self,
        entry: &ModelEntry,
        result: Result<
            Result<crate::provider::SseStream, crate::provider::ProviderError>,
            tokio::time::error::Elapsed,
        >,
    ) -> Result<OutgoingEventStream, ()> {
        match result {
            Ok(Ok(sse_stream)) => {
                self.cooldown
                    .record_success(&entry.provider, &entry.model)
                    .await;
                let incoming: IncomingSseStream = Box::pin(ReceiverStream::new(sse_stream));
                let machine = self.protocol.create_sse_machine();
                let stream = self.protocol.parse_sse_stream(incoming, machine).await;
                Ok(stream)
            }
            Ok(Err(provider_err)) => {
                let llm_err = LLMError::ApiError(provider_err.to_string());
                let kind = llm_err.kind();
                tracing::warn!(
                    provider = %entry.provider,
                    model = %entry.model,
                    error = %llm_err,
                    kind = ?kind,
                    "fallback streaming call failed"
                );
                self.cooldown
                    .record_failure(&entry.provider, &entry.model, kind)
                    .await;
                Err(())
            }
            Err(_elapsed) => {
                tracing::warn!(
                    provider = %entry.provider,
                    model = %entry.model,
                    "fallback streaming call timed out"
                );
                self.cooldown
                    .record_failure(&entry.provider, &entry.model, ErrorKind::Transient)
                    .await;
                Err(())
            }
        }
    }
}

// --- Degraded streaming fallback ---

impl FallbackClient {
    /// All streaming entries failed — degrade to non-streaming.
    ///
    /// Walks the chain without cooldown checks (the streaming cooldown
    /// should not block a non-streaming attempt) and wraps the first
    /// successful response as a character-by-character stream.
    async fn degraded_stream(
        &self,
        mut request: InternalRequest,
    ) -> Result<OutgoingEventStream, LLMError> {
        tracing::warn!("all streaming entries failed, degrading to non-streaming");
        for entry in &self.fallback_chain {
            let provider = match self.registry.get(&entry.provider).await {
                Some(p) => p,
                None => continue,
            };
            request.model = entry.model.clone();
            let chat_request = ChatRequest {
                model: request.model.clone(),
                messages: request
                    .messages
                    .iter()
                    .map(|m| crate::Message {
                        role: m.role.clone(),
                        content: m.content.clone(),
                    })
                    .collect(),
                temperature: request.temperature,
                max_tokens: request.max_tokens,
            };
            match self.call_provider_unified(&provider, chat_request).await {
                Ok(response) => {
                    return Ok(crate::unified_fallback::response_to_stream(response));
                }
                Err(err) => {
                    tracing::warn!(
                        provider = %entry.provider,
                        model = %entry.model,
                        error = %err,
                        "degraded non-streaming call also failed"
                    );
                }
            }
        }
        Err(LLMError::ApiError(
            "all models in fallback chain exhausted".to_string(),
        ))
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
