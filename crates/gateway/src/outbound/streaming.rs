//! Streaming-mode outbound entries and the core dispatch loop
//! (pre-flight middleware → incremental event loop → finish pipeline →
//! checkpoint persistence). Incremental-phase event handling lives in
//! [`streaming_incremental`]; the post-stream finish pipeline lives in
//! [`streaming_finalize`].

use crate::outbound_helpers::{StreamContext, StreamState};
use crate::outbound_middleware::runner::run_pre_flight_check;
use crate::{Gateway, GatewayError};
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_llm::types::StreamEvent;

use super::types::{OutboundMeta, StreamResult};

impl Gateway {
    /// Send a streaming LLM response via the registered IM plugin.
    ///
    /// Delegates to `send_outbound_streaming_inner` for core logic.
    pub async fn send_outbound_streaming<E: std::fmt::Display>(
        &self,
        session_id: &str,
        channel: &str,
        stream: impl futures::Stream<Item = Result<StreamEvent, E>> + Unpin,
        plugin: &std::sync::Arc<dyn IMPlugin>,
        meta: OutboundMeta,
    ) -> Result<StreamResult, GatewayError> {
        self.send_outbound_streaming_inner(session_id, channel, stream, plugin, meta)
            .await
    }

    /// Streaming outbound dispatch with session-assembled content blocks.
    ///
    /// When `session_content_blocks` is provided, the post-stream pipeline
    /// uses them as the source of truth instead of internal StreamState.
    pub async fn send_outbound_streaming_assembled<E: std::fmt::Display>(
        &self,
        session_id: &str,
        channel: &str,
        stream: impl futures::Stream<Item = Result<StreamEvent, E>> + Unpin,
        plugin: &std::sync::Arc<dyn IMPlugin>,
        meta: OutboundMeta,
    ) -> Result<StreamResult, GatewayError> {
        self.send_outbound_streaming_inner(session_id, channel, stream, plugin, meta)
            .await
    }
}

impl Gateway {
    /// Core streaming outbound dispatch.
    ///
    /// Drives a [`DefaultStreamingRenderer`] over the [`StreamEvent`] stream,
    /// dispatching incremental output to `plugin` as it becomes available:
    /// - Text delta → line buffer → complete lines → `plugin.send` (text)
    /// - BlockEnd (non-Text) → `plugin.render(&[block], None)` → `plugin.send`
    /// - MessageEnd → flush remaining content → `plugin.send`
    ///
    /// When `session_blocks` is provided, the post-stream pipeline uses
    /// those session-assembled `ContentBlock`s instead of the internal
    /// `StreamState` accumulation.
    async fn send_outbound_streaming_inner<E: std::fmt::Display>(
        &self,
        session_id: &str,
        channel: &str,
        mut stream: impl futures::Stream<Item = Result<StreamEvent, E>> + Unpin,
        plugin: &std::sync::Arc<dyn IMPlugin>,
        meta: OutboundMeta,
    ) -> Result<StreamResult, GatewayError> {
        let session_blocks = if meta.session_content_blocks.is_empty() {
            None
        } else {
            Some((
                meta.session_content_blocks.clone(),
                meta.session_usage.clone(),
            ))
        };
        let chat_id = self.resolve_outbound_chat_id(session_id).await?;

        // Resolve thread routing from session checkpoint for outbound
        // thread routing.
        let routing = self.resolve_outbound_thread_routing(session_id).await;

        let verbosity_level = self.resolve_outbound_verbosity_level(session_id).await;

        // Pre-flight middleware check: execute once before the stream loop
        // instead of per-chunk.
        self.run_streaming_pre_flight(session_id, channel, &chat_id)
            .await?;

        let mut state = StreamState::new(verbosity_level);
        let processor_registry = self.processor_registry.read().unwrap().clone();
        let ctx = StreamContext {
            gateway: self,
            plugin,
            session_id,
            channel,
            chat_id: &chat_id,
            thread_id: routing.thread_id.as_deref(),
            reply_ref: routing.reply_ref.as_deref(),
            registry: processor_registry.as_ref(),
            trace_id: meta.trace_id.as_deref(),
            session_key: meta.session_key.as_deref(),
        };
        self.run_streaming_event_loop(&ctx, &mut state, &mut stream)
            .await?;
        tracing::debug!(session_id, channel, "streaming outbound complete");

        let result = self
            .finish_streaming_pipeline(session_blocks, state, channel, session_id, verbosity_level)
            .await?;

        self.persist_streaming_checkpoint(channel, session_id, &chat_id, &result)
            .await;

        Ok(result)
    }
}

impl Gateway {
    /// Pre-flight middleware check: execute once before the stream loop
    /// instead of per-chunk. This avoids middleware overhead on every
    /// incremental update and aligns with the design doc requirement
    /// that streaming pre-flight runs exactly once using session metadata.
    ///
    /// On rejection, logs the reason and sends a rejection notification
    /// via the simplified outbound path (skips middleware to avoid
    /// re-rejection).
    async fn run_streaming_pre_flight(
        &self,
        session_id: &str,
        channel: &str,
        chat_id: &str,
    ) -> Result<(), GatewayError> {
        let middlewares = self.get_outbound_middlewares().await;
        if middlewares.is_empty() {
            return Ok(());
        }
        let mctx = Self::make_middleware_ctx(session_id, channel, chat_id);
        if let Err(e) = run_pre_flight_check(&middlewares, &mctx).await {
            // Log the rejection reason at warning level.
            match &e {
                closeclaw_common::MiddlewareError::Rejected { name, reason } => {
                    tracing::warn!(
                        middleware = %name,
                        reason = %reason,
                        session_id,
                        "pre-flight middleware rejected streaming outbound"
                    );
                }
                closeclaw_common::MiddlewareError::MiddlewareFailed { name, .. } => {
                    tracing::warn!(
                        middleware = %name,
                        session_id,
                        "pre-flight middleware failed during streaming outbound"
                    );
                }
            }
            let _ = self
                .send_outbound_simplified(
                    chat_id,
                    channel,
                    "Your message was not sent due to an outbound policy restriction.",
                )
                .await;
            return Err(GatewayError::OutboundError(format!(
                "pre-flight middleware rejected streaming: {}",
                e
            )));
        }
        Ok(())
    }
}
