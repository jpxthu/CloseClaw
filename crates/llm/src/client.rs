//! LLM UnifiedChatClient — assembles the full Provider → Protocol → Interpreter call chain.
//!
//! [`UnifiedChatClient`] is the single unified entry point for sending chat requests
//! through the LLM framework. It owns the four pillars of the call pipeline:
//! - **Provider** — HTTP transport (OpenAI, Anthropic, GLM, DeepSeek, …)
//! - **ChatProtocol** — request serialisation / response deserialisation
//! - **InterpreterRegistry** — maps `(provider_id, model)` → [`ModelInterpreter`]
//! - **PluginPipeline** — before-request / after-response / on-stream-event hooks
//!
//! The non-streaming [`chat`](UnifiedChatClient::chat) flow is:
//! ```ignore
//! PluginPipeline.before_request
//!   → Interpreter.inject_extra_body
//!     → ChatProtocol.build_request
//!       → Provider.send
//!         → Interpreter.interpret_response
//!           → PluginPipeline.after_response
//! ```

use std::sync::Arc;

use futures::StreamExt;

use crate::cache_adapter::CacheAdapter;
use crate::interpreter::InterpreterRegistry;
use crate::plugin::PluginPipeline;
use crate::protocol::{ChatProtocol, IncomingSseStream, OutgoingEventStream, ProtocolError};
use crate::provider::{Provider, SseStream};
use crate::types::{InternalRequest, RawSseChunk, StreamEvent, UnifiedResponse};

/// Unified client error — covers all failures in the call pipeline.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("provider error: {0}")]
    Provider(#[from] crate::provider::ProviderError),
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
}

/// Result type alias for client operations.
pub type Result<T> = std::result::Result<T, ClientError>;

// ─────────────────────────────────────────────────────────────────────────────
// ReceiverStream — bridges tokio mpsc::Receiver → futures::Stream
// ─────────────────────────────────────────────────────────────────────────────

/// A lightweight [`futures::Stream`] adapter over
/// [`tokio::sync::mpsc::Receiver`].
///
/// `tokio::sync::mpsc::Receiver` does not implement `futures::Stream` directly.
/// This wrapper bridges the gap by delegating [`StreamExt::poll_next`] to
/// [`Receiver::poll_recv`].
struct ReceiverStream {
    rx: Option<SseStream>,
}

impl ReceiverStream {
    fn new(rx: SseStream) -> Self {
        Self { rx: Some(rx) }
    }
}

impl futures::Stream for ReceiverStream {
    type Item = RawSseChunk;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.rx.as_mut() {
            Some(rx) => rx.poll_recv(cx),
            None => std::task::Poll::Ready(None),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UnifiedChatClient
// ─────────────────────────────────────────────────────────────────────────────

/// The unified entry point for all LLM chat requests.
///
/// Holds a configured provider, protocol, interpreter registry, and plugin
/// pipeline, and exposes [`chat`](UnifiedChatClient::chat) /
/// [`chat_streaming`](UnifiedChatClient::chat_streaming) methods that execute
/// the full call chain.
pub struct UnifiedChatClient {
    provider: Arc<dyn Provider>,
    protocol: Arc<dyn ChatProtocol>,
    interpreter_registry: Arc<InterpreterRegistry>,
    plugin_pipeline: Arc<PluginPipeline>,
    cache_adapter: Arc<dyn CacheAdapter>,
}

impl std::fmt::Debug for UnifiedChatClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedChatClient")
            .field("provider", &self.provider.id())
            .field("protocol", &self.protocol.protocol_id())
            .field("cache_adapter", &self.cache_adapter.name())
            .finish()
    }
}

impl UnifiedChatClient {
    /// Creates a new `UnifiedChatClient` from its components including a
    /// cache adapter.
    pub fn new(
        provider: Arc<dyn Provider>,
        protocol: Arc<dyn ChatProtocol>,
        interpreter_registry: InterpreterRegistry,
        plugin_pipeline: PluginPipeline,
        cache_adapter: Arc<dyn CacheAdapter>,
    ) -> Self {
        Self {
            provider,
            protocol,
            interpreter_registry: Arc::new(interpreter_registry),
            plugin_pipeline: Arc::new(plugin_pipeline),
            cache_adapter,
        }
    }

    /// Convenience constructor that uses [`NoopCacheAdapter`](crate::cache_adapter::NoopCacheAdapter)
    /// for backward compatibility.
    pub fn with_noop_cache_adapter(
        provider: Arc<dyn Provider>,
        protocol: Arc<dyn ChatProtocol>,
        interpreter_registry: InterpreterRegistry,
        plugin_pipeline: PluginPipeline,
    ) -> Self {
        Self::new(
            provider,
            protocol,
            interpreter_registry,
            plugin_pipeline,
            Arc::new(crate::cache_adapter::NoopCacheAdapter),
        )
    }

    /// Returns the provider identifier.
    pub fn provider_id(&self) -> &str {
        self.provider.id()
    }

    // ── Non-streaming chat ──────────────────────────────────────────────────

    /// Sends a single, non-streaming chat request through the full pipeline.
    ///
    /// # Pipeline steps (in order)
    /// 1. **PluginPipeline.before_request** — each plugin may mutate the request.
    /// 2. **Interpreter.inject_extra_body** — provider-specific field injection.
    /// 3. **ChatProtocol.build_request** — serialises the request to a JSON body.
    /// 4. **Provider.send** — performs the HTTP request.
    /// 5. **Interpreter.interpret_response** — normalises the internal response.
    /// 6. **PluginPipeline.after_response** — each plugin may mutate the final
    ///    [`UnifiedResponse`] before it is returned.
    pub async fn chat(&self, mut request: InternalRequest) -> Result<UnifiedResponse> {
        let model = request.model.clone();
        let provider_id = self.provider.id();
        self.cache_adapter.apply(&mut request);
        self.plugin_pipeline.before_request(&mut request);
        let interpreter = self.interpreter_registry.resolve(provider_id, &model);
        interpreter.inject_extra_body(&mut request);
        let body = self
            .protocol
            .build_request(&request)
            .map_err(ClientError::Protocol)?;
        let internal_response = self
            .provider
            .send(request, body)
            .await
            .map_err(ClientError::Provider)?;
        let mut response = interpreter.interpret_response(internal_response);
        self.plugin_pipeline.after_response(&mut response);
        Ok(response)
    }

    // ── Streaming chat ──────────────────────────────────────────────────────

    /// Sends a streaming chat request through the full pipeline.
    ///
    /// Unlike [`chat`](UnifiedChatClient::chat), this returns a stream of
    /// [`StreamEvent`] values rather than a single [`UnifiedResponse`].
    ///
    /// # Pipeline steps (in order)
    /// 1. **PluginPipeline.before_request** — mutates the request.
    /// 2. **Interpreter.inject_extra_body** — provider-specific field injection.
    /// 3. **ChatProtocol.build_request** — serialises with `stream: true`.
    /// 4. **Provider.send_streaming** — returns a raw SSE channel.
    /// 5. **ChatProtocol.parse_sse_stream** — parses SSE chunks into events.
    /// 6. **Interpreter.interpret_stream_event** — normalises each event.
    /// 7. **PluginPipeline.on_stream_event** — each plugin may forward, modify, or suppress each event.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] for setup/transport errors. Streaming parse
    /// errors are emitted as [`StreamEvent::Error`] within the stream itself.
    pub async fn chat_streaming(
        &self,
        mut request: InternalRequest,
    ) -> Result<OutgoingEventStream> {
        let model = request.model.clone();
        let provider_id = self.provider.id();
        self.cache_adapter.apply(&mut request);
        self.plugin_pipeline.before_request(&mut request);
        let interpreter = self.interpreter_registry.resolve(provider_id, &model);
        interpreter.inject_extra_body(&mut request);
        request.stream = true;
        let body = self
            .protocol
            .build_request(&request)
            .map_err(ClientError::Protocol)?;
        let sse_stream: SseStream = self
            .provider
            .send_streaming(request, body)
            .await
            .map_err(ClientError::Provider)?;
        let machine = self.protocol.create_sse_machine();
        let incoming: IncomingSseStream = Box::pin(ReceiverStream::new(sse_stream));
        let event_stream = self.protocol.parse_sse_stream(incoming, machine).await;
        let registry = Arc::clone(&self.interpreter_registry);
        let pipeline = Arc::clone(&self.plugin_pipeline);
        Ok(process_event_stream(
            event_stream,
            registry,
            provider_id.to_string(),
            model,
            pipeline,
        ))
    }
}

/// Applies interpreter normalisation and plugin pipeline hooks to each event.
fn process_event_stream(
    event_stream: OutgoingEventStream,
    registry: Arc<InterpreterRegistry>,
    provider_id: String,
    model: String,
    pipeline: Arc<PluginPipeline>,
) -> OutgoingEventStream {
    Box::pin(async_stream::try_stream! {
        let mut stream = event_stream;
        let interpreter = registry.resolve(&provider_id, &model);
        while let Some(event_result) = stream.next().await {
            let event = match event_result {
                Ok(e) => e,
                Err(e) => { yield StreamEvent::Error { message: e.to_string() }; continue; }
            };
            let normalised = match interpreter.interpret_stream_event(event) {
                Some(e) => e,
                None => continue,
            };
            let final_event = match pipeline.on_stream_event(&normalised) {
                Some(e) => e,
                None => continue,
            };
            yield final_event;
        }
    })
}
