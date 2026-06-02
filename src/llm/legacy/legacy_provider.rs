//! [`LegacyProviderBridge`] wraps an old-style [`LLMProvider`] into the new [`Provider`] trait.
//!
//! Bridges the legacy interface (`ChatRequest`/`ChatResponse`/`Message`) to the
//! new architecture (`InternalRequest`/`InternalResponse`/`InternalMessage`).

#![allow(deprecated)]

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;

use crate::llm::provider::{Provider, ProviderError, Result, SseStream};
use crate::llm::types::{
    InternalRequest, InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage,
};
use crate::llm::{ChatRequest, ChatResponse, ChatStreamChunk, LLMProvider, Message};

/// Bridge wrapping an old-style [`LLMProvider`] into the new [`Provider`] trait.
///
/// `send()` converts [`InternalRequest`] → [`ChatRequest`], delegates to
/// `inner.chat()`, then converts [`ChatResponse`] → [`InternalResponse`].
/// `send_streaming()` does the same via `inner.chat_streaming()`, wrapping
/// each [`ChatStreamChunk`] into a [`RawSseChunk`].
pub struct LegacyProviderBridge<P> {
    inner: P,
    base_url: String,
    api_key: String,
    supported_protocols: Vec<ProtocolId>,
    http_client: Client,
    default_headers: HeaderMap,
}

impl<P: LLMProvider> LegacyProviderBridge<P> {
    /// Creates a new adapter wrapping `inner` with the given configuration.
    pub fn new(
        inner: P,
        base_url: String,
        api_key: String,
        supported_protocols: Vec<ProtocolId>,
        http_client: Client,
        default_headers: HeaderMap,
    ) -> Self {
        Self {
            inner,
            base_url,
            api_key,
            supported_protocols,
            http_client,
            default_headers,
        }
    }

    /// Builds a [`ChatRequest`] from an [`InternalRequest`].
    fn build_chat_request(request: &InternalRequest) -> ChatRequest {
        let messages: Vec<Message> = request
            .messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();
        ChatRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        }
    }

    /// Converts a [`ChatResponse`] into an [`InternalResponse`].
    fn to_internal_response(response: ChatResponse) -> InternalResponse {
        InternalResponse {
            content_blocks: vec![RawContentBlock::Text(response.content)],
            usage: RawUsage {
                prompt_tokens: response.usage.prompt_tokens,
                completion_tokens: response.usage.completion_tokens,
                total_tokens: Some(response.usage.total_tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        }
    }

    /// Converts a [`ChatStreamChunk`] into an optional [`RawSseChunk`].
    fn chunk_to_raw(chunk: ChatStreamChunk) -> Option<RawSseChunk> {
        match chunk {
            ChatStreamChunk::Text(text) => Some(RawSseChunk {
                event_type: "message".into(),
                data: text,
            }),
            ChatStreamChunk::Done { model, usage } => {
                let json = serde_json::json!({
                    "type": "message_end",
                    "model": model,
                    "usage": {
                        "prompt_tokens": usage.prompt_tokens,
                        "completion_tokens": usage.completion_tokens,
                        "total_tokens": usage.total_tokens,
                    }
                });
                Some(RawSseChunk {
                    event_type: "message".into(),
                    data: json.to_string(),
                })
            }
            ChatStreamChunk::Error(err) => Some(RawSseChunk {
                event_type: "error".into(),
                data: err.to_string(),
            }),
        }
    }
}

#[async_trait]
impl<P: LLMProvider> Provider for LegacyProviderBridge<P> {
    fn id(&self) -> &str {
        self.inner.name()
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn supported_protocols(&self) -> &[ProtocolId] {
        &self.supported_protocols
    }

    fn http_client(&self) -> &Client {
        &self.http_client
    }

    fn default_headers(&self) -> &HeaderMap {
        &self.default_headers
    }

    async fn send(
        &self,
        request: InternalRequest,
        _body: serde_json::Value,
    ) -> Result<InternalResponse> {
        let chat_req = Self::build_chat_request(&request);
        let response = self
            .inner
            .chat(chat_req)
            .await
            .map_err(|e| ProviderError::Legacy(e.to_string()))?;
        Ok(Self::to_internal_response(response))
    }

    async fn send_streaming(
        &self,
        request: InternalRequest,
        _body: serde_json::Value,
    ) -> Result<SseStream> {
        let chat_req = Self::build_chat_request(&request);
        let streaming_rx = self
            .inner
            .chat_streaming(chat_req)
            .await
            .map_err(|e| ProviderError::Legacy(e.to_string()))?;
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            let mut rx = streaming_rx;
            while let Some(chunk) = rx.recv().await {
                if let Some(raw) = Self::chunk_to_raw(chunk) {
                    if tx.send(raw).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }
}

// Tests for LegacyProviderBridge — compiled only when the fake-llm feature is enabled.
// This mirrors the pattern used by FakeProvider (fake.rs → fake_tests.rs).
#[cfg(all(test, feature = "fake-llm"))]
#[path = "legacy_provider_tests.rs"]
mod provider_tests;

#[cfg(all(test, not(feature = "fake-llm")))]
mod provider_tests {
    // No-op placeholder so the module compiles without the fake-llm feature.
    // Real tests live in legacy_provider_tests.rs which is only compiled
    // when the feature is active.
}
