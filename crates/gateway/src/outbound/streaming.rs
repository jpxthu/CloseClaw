//! Streaming-mode outbound entries and the core dispatch loop
//! (pre-flight middleware → incremental event loop → finish pipeline →
//! checkpoint persistence). Incremental-phase event handling lives in
//! [`streaming_incremental`]; the post-stream finish pipeline lives in
//! [`streaming_finalize`].

use crate::outbound_helpers::{dispatch_text, CheckpointMeta, StreamContext, StreamState};
use crate::outbound_middleware::runner::run_pre_flight_check;
use crate::{Gateway, GatewayError};
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::LlmState;
use closeclaw_llm::types::{ContentBlock, StreamEvent};
use futures::StreamExt;

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
        // instead of per-chunk. This avoids middleware overhead on every
        // incremental update and aligns with the design doc requirement
        // that streaming pre-flight runs exactly once using session metadata.
        let middlewares = self.get_outbound_middlewares().await;
        if !middlewares.is_empty() {
            let mctx = Self::make_middleware_ctx(session_id, channel, &chat_id);
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
                // Send a rejection notification to the user via simplified path
                // (skips middleware to avoid re-rejection).
                let _ = self
                    .send_outbound_simplified(
                        &chat_id,
                        channel,
                        "Your message was not sent due to an outbound policy restriction.",
                    )
                    .await;
                return Err(GatewayError::OutboundError(format!(
                    "pre-flight middleware rejected streaming: {}",
                    e
                )));
            }
        }

        let mut state = StreamState::new(verbosity_level);
        let mut first_event_received = false;
        let timeout_duration = std::time::Duration::from_millis(200);
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
        loop {
            tokio::select! {
                event_result = stream.next() => {
                    let Some(event_result) = event_result else {
                        break;
                    };
                    let event = event_result
                        .map_err(|e| GatewayError::OutboundError(e.to_string()))?;
                    // Transition LlmState from Requesting → Receiving on the first
                    // stream event. This aligns the runtime state machine with the
                    // design doc: Idle → Requesting → Receiving → Idle.
                    if !first_event_received {
                        first_event_received = true;
                        if let Some(cs) = self
                            .session_manager
                            .get_conversation_session(session_id)
                            .await
                        {
                            cs.read().await.set_llm_state(LlmState::Receiving);
                        }
                    }
                    self.process_stream_event(&ctx, event, &mut state).await?;
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    // Timeout check: force-output any buffered content.
                    let out = ctx.plugin.check_stream_timeout();
                    if !out.text_messages.is_empty() {
                        dispatch_text(&ctx, out, &mut state).await?;
                    }
                }
            }
        }
        tracing::debug!(session_id, channel, "streaming outbound complete");

        let result = self
            .finish_streaming_pipeline(session_blocks, state, channel, session_id, verbosity_level)
            .await?;

        // Persist streaming outbound checkpoint — mirrors the batch path
        // in `dispatch_and_persist`. Without this, a crash after streaming
        // completes would lose the outbound history.
        let text = result
            .content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        let content_blocks_json = serde_json::to_string(&result.content_blocks).unwrap_or_default();
        let msg_id = format!("out-{}", chrono::Utc::now().timestamp_millis());
        let meta = CheckpointMeta {
            platform: Some(channel.to_string()),
            dsl_result: result.dsl_result.clone(),
            content_blocks: Some(content_blocks_json),
        };
        let msg = Self::make_outbound_msg(channel, chat_id, msg_id, text, meta);
        crate::outbound_helpers::persist_outbound_checkpoint(self, session_id, &msg, true).await;

        Ok(result)
    }
}
