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
use crate::protocol::{OutgoingEventStream, ProtocolError};
use crate::retry::CooldownManager;
use crate::types::{InternalRequest, UnifiedResponse};
use crate::LLMError;
use closeclaw_common::processor::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent};
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
    pub fn primary(&self) -> &Arc<UnifiedChatClient> {
        &self.chain.first().expect("chain must not be empty").client
    }

    /// Returns the primary provider's default header key-value pairs.
    ///
    /// Delegates to [`UnifiedChatClient::default_header_pairs`] on the
    /// primary (first) chain entry. Returns an empty `Vec` when the
    /// chain is empty (e.g. in test fixtures).
    pub fn default_header_pairs(&self) -> Vec<(String, String)> {
        self.chain
            .first()
            .map(|entry| entry.client.default_header_pairs())
            .unwrap_or_default()
    }

    /// Send a streaming chat request through the fallback chain.
    ///
    /// Walks the chain trying `chat_streaming` on each entry, skipping
    /// cooldown entries. On success returns the stream; on failure records
    /// cooldown and tries the next entry. If every entry's streaming call
    /// fails, degrades to a non-streaming [`chat`](Self::chat) and wraps
    /// the complete response as a single-chunk stream.
    pub async fn chat_streaming(
        &self,
        mut request: InternalRequest,
    ) -> Result<OutgoingEventStream, ClientError> {
        let mut idx = 0;
        loop {
            match self.chain.get(idx) {
                None => {
                    // All streaming entries exhausted — degrade to non-streaming.
                    return self.degraded_stream(request).await;
                }
                Some(entry) => {
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

                    match entry.client.chat_streaming(request.clone()).await {
                        Ok(stream) => {
                            self.cooldown
                                .record_success(&entry.provider_id, &entry.model_id)
                                .await;
                            return Ok(stream);
                        }
                        Err(client_err) => {
                            let llm_err: LLMError = client_err.into();
                            let kind = llm_err.kind();
                            tracing::warn!(
                                provider = %entry.provider_id,
                                model = %entry.model_id,
                                error = %llm_err,
                                kind = ?kind,
                                "unified fallback streaming call failed"
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
    }

    /// All streaming entries failed — degrade to non-streaming.
    ///
    /// Walks the chain without cooldown checks (the streaming cooldown
    /// should not block a non-streaming attempt) and wraps the first
    /// successful response as a single-chunk [`OutgoingEventStream`].
    async fn degraded_stream(
        &self,
        mut request: InternalRequest,
    ) -> Result<OutgoingEventStream, ClientError> {
        tracing::warn!("all streaming entries failed, degrading to non-streaming");
        for entry in &self.chain {
            request.model = entry.model_id.clone();
            match entry.client.chat(request.clone()).await {
                Ok(response) => return Ok(response_to_stream(response)),
                Err(client_err) => {
                    let llm_err: LLMError = client_err.into();
                    tracing::warn!(
                        provider = %entry.provider_id,
                        model = %entry.model_id,
                        error = %llm_err,
                        "degraded non-streaming call also failed"
                    );
                }
            }
        }
        Err(ClientError::Protocol(ProtocolError::ResponseParse(
            "all models in unified fallback chain exhausted".to_string(),
        )))
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
// Degraded-stream helper
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a [`UnifiedResponse`] into a single-chunk [`OutgoingEventStream`].
///
/// Each [`ContentBlock`] in the response becomes a `BlockStart → BlockDelta
/// → BlockEnd` triple, followed by a `MessageEnd` with usage stats.
pub(crate) fn response_to_stream(response: UnifiedResponse) -> OutgoingEventStream {
    use futures::stream;

    let mut events: Vec<Result<StreamEvent, ProtocolError>> = Vec::new();
    for (i, block) in response.content_blocks.iter().enumerate() {
        let block_type = match block {
            ContentBlock::Text(_) => ContentBlockType::Text,
            ContentBlock::Thinking { .. } => ContentBlockType::Thinking,
            ContentBlock::ToolUse { .. } => ContentBlockType::ToolUse,
            ContentBlock::ToolResult { .. } => continue,
            ContentBlock::Image { .. } => continue,
            ContentBlock::Audio { .. } => continue,
            ContentBlock::File { .. } => continue,
        };
        events.push(Ok(StreamEvent::BlockStart {
            index: i,
            block_type,
        }));
        // For Text blocks, emit one BlockDelta per character (typewriter effect).
        // For other block types, emit a single BlockDelta as before.
        match block {
            ContentBlock::Text(text) => {
                for ch in text.chars() {
                    events.push(Ok(StreamEvent::BlockDelta {
                        index: i,
                        delta: ContentDelta::Text {
                            text: ch.to_string(),
                        },
                    }));
                }
            }
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                events.push(Ok(StreamEvent::BlockDelta {
                    index: i,
                    delta: ContentDelta::Thinking {
                        thinking: thinking.clone(),
                        signature: signature.clone(),
                    },
                }));
            }
            ContentBlock::ToolUse { id, name, input } => {
                events.push(Ok(StreamEvent::BlockDelta {
                    index: i,
                    delta: ContentDelta::ToolUseInputChunk {
                        input: serde_json::json!({
                            "id": id,
                            "name": name,
                            "input": input
                        })
                        .to_string(),
                    },
                }));
            }
            ContentBlock::ToolResult { .. }
            | ContentBlock::Image { .. }
            | ContentBlock::Audio { .. }
            | ContentBlock::File { .. } => unreachable!(),
        };
        events.push(Ok(StreamEvent::BlockEnd {
            index: i,
            block_type,
        }));
    }
    events.push(Ok(StreamEvent::MessageEnd {
        usage: Some(response.usage),
        finish_reason: response.finish_reason,
    }));
    Box::pin(stream::iter(events))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
