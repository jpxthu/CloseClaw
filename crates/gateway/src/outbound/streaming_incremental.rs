//! Streaming incremental-phase event handling: the incremental event
//! loop orchestration, per-[`StreamEvent`] dispatch, plus the helpers
//! that render and dispatch each event's output through the plugin
//! (non-text blocks additionally pass through the incremental
//! Processor Chain).

use crate::outbound_helpers::{
    dispatch_text, process_single_through_chain, send_render_block, StreamContext, StreamState,
};
use crate::{Gateway, GatewayError};
use closeclaw_common::{LlmState, VerbosityLevel};
use closeclaw_llm::types::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedUsage,
};
use futures::StreamExt;

impl Gateway {
    /// Incremental-phase event loop: polls the LLM event stream against a
    /// 200ms timeout tick, dispatching each [`StreamEvent`] via
    /// [`Gateway::process_stream_event`] and force-flushing buffered lines
    /// on timeout. Returns when the stream ends or an event fails.
    pub(super) async fn run_streaming_event_loop<E: std::fmt::Display>(
        &self,
        ctx: &StreamContext<'_>,
        state: &mut StreamState,
        stream: &mut (impl futures::Stream<Item = Result<StreamEvent, E>> + Unpin),
    ) -> Result<(), GatewayError> {
        let timeout_duration = std::time::Duration::from_millis(200);
        let mut first_event_received = false;
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
                            .get_conversation_session(ctx.session_id)
                            .await
                        {
                            cs.read().await.set_llm_state(LlmState::Receiving);
                        }
                    }
                    self.process_stream_event(ctx, event, state).await?;
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    // Timeout check: force-output any buffered content.
                    let out = ctx.plugin.check_stream_timeout();
                    if !out.text_messages.is_empty() {
                        dispatch_text(ctx, out, state).await?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Gateway {
    /// Process a single [`StreamEvent`] and update `state`.
    ///
    /// Split from `send_outbound_streaming` to keep the main loop under the
    /// 50-line helper cap. Each arm delegates to a dedicated helper to stay
    /// within the 50-line function body limit.
    async fn process_stream_event(
        &self,
        ctx: &StreamContext<'_>,
        event: StreamEvent,
        state: &mut StreamState,
    ) -> Result<(), GatewayError> {
        match event {
            StreamEvent::BlockDelta { index, delta } => {
                self.handle_block_delta(ctx, index, delta, state).await?;
            }
            StreamEvent::BlockEnd { block_type, .. } => {
                // Thinking indicator: send stop signal before rendering.
                if block_type == ContentBlockType::Thinking
                    && state.verbosity_level != VerbosityLevel::Off
                {
                    ctx.plugin.send_thinking_indicator(false);
                }
                self.handle_block_end(ctx, event, block_type, state).await?;
            }
            StreamEvent::MessageEnd { usage, .. } => {
                self.handle_message_end(ctx, usage, state).await?;
            }
            StreamEvent::Error { message } => {
                // Error → no incremental output per design doc.
                tracing::warn!(
                    session_id = ctx.chat_id,
                    error = %message,
                    "streaming error"
                );
                return Err(GatewayError::StreamError {
                    message,
                    partial_content: std::mem::take(&mut state.error_partial_blocks),
                });
            }
            StreamEvent::BlockStart { index, block_type } => {
                // Thinking indicator: send start signal on Thinking BlockStart.
                if block_type == ContentBlockType::Thinking
                    && state.verbosity_level != VerbosityLevel::Off
                {
                    ctx.plugin.send_thinking_indicator(true);
                }
                ctx.plugin
                    .handle_stream_event(StreamEvent::BlockStart { index, block_type });
            }
        }
        Ok(())
    }

    /// Handle a [`StreamEvent::BlockDelta`]: delegate to the plugin and
    /// dispatch any completed text lines.
    async fn handle_block_delta(
        &self,
        ctx: &StreamContext<'_>,
        index: usize,
        delta: ContentDelta,
        state: &mut StreamState,
    ) -> Result<(), GatewayError> {
        // Accumulate Image/Audio/File deltas at Gateway level.
        if let ContentDelta::ImageRef { name, url }
        | ContentDelta::AudioRef { name, url }
        | ContentDelta::FileRef { name, url } = &delta
        {
            state.media_name = Some(name.clone());
            state.media_url = Some(url.clone());
            return Ok(());
        }
        let is_text_delta = matches!(delta, ContentDelta::Text { .. });
        let out = ctx
            .plugin
            .handle_stream_event(StreamEvent::BlockDelta { index, delta });
        // Text blocks are never filtered by verbosity — only Thinking and
        // other non-Text blocks are filtered at BlockEnd.
        if is_text_delta {
            dispatch_text(ctx, out, state).await?;
        }
        Ok(())
    }
}

impl Gateway {
    /// Handle a [`StreamEvent::BlockEnd`]: send non-text render blocks
    /// (after incremental chain processing) and dispatch remaining text.
    /// Non-Text, non-media blocks are processed through
    /// [`ProcessorChain::process_outbound_incremental`] before dispatch,
    /// so VerbosityFilter executes uniformly via the chain.
    async fn handle_block_end(
        &self,
        ctx: &StreamContext<'_>,
        event: StreamEvent,
        block_type: ContentBlockType,
        state: &mut StreamState,
    ) -> Result<(), GatewayError> {
        let mut out = ctx.plugin.handle_stream_event(event);
        if block_type != ContentBlockType::Text {
            if matches!(
                block_type,
                ContentBlockType::Image | ContentBlockType::Audio | ContentBlockType::File
            ) {
                let block = state.take_media_block(block_type);
                state.content_blocks.push(block);
            } else {
                let render_blocks = std::mem::take(&mut out.render_blocks);
                self.process_and_send_non_text_blocks(ctx, &render_blocks, block_type, state)
                    .await?;
            }
        }

        dispatch_text(ctx, out, state).await
    }

    /// Process non-text render blocks through the incremental chain
    /// ([`ProcessorChain::process_outbound_incremental`]) before dispatch
    /// and storage. Falls back to direct send when registry is absent.
    async fn process_and_send_non_text_blocks(
        &self,
        ctx: &StreamContext<'_>,
        render_blocks: &[ContentBlock],
        block_type: ContentBlockType,
        state: &mut StreamState,
    ) -> Result<(), GatewayError> {
        for block in render_blocks {
            // Track original blocks for error-path preservation before
            // VerbosityFilter runs (design doc §流式输出: complete Thinking
            // blocks are retained in conversation history on streaming error).
            state.error_partial_blocks.push(block.clone());
            if let Some(registry) = ctx.registry {
                match process_single_through_chain(registry.as_ref(), block, state.verbosity_level)
                    .await
                {
                    Ok(processed_blocks) => {
                        for processed_block in &processed_blocks {
                            send_render_block(ctx, processed_block).await?;
                        }
                        state.content_blocks.extend(processed_blocks);
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            block_type = ?block_type,
                            "incremental chain failed for non-text block, sending original"
                        );
                        send_render_block(ctx, block).await?;
                        state.content_blocks.push(block.clone());
                    }
                }
            } else {
                send_render_block(ctx, block).await?;
                state.content_blocks.push(block.clone());
            }
        }
        Ok(())
    }

    /// Handle a [`StreamEvent::MessageEnd`]: flush the stream and update
    /// token usage. Non-text render blocks were already sent at BlockEnd.
    async fn handle_message_end(
        &self,
        ctx: &StreamContext<'_>,
        usage: Option<UnifiedUsage>,
        state: &mut StreamState,
    ) -> Result<(), GatewayError> {
        let mut out = ctx.plugin.flush_stream();
        // Non-text render_blocks were already sent in BlockEnd;
        // discard them here to avoid duplicate sends.
        out.render_blocks.clear();
        dispatch_text(ctx, out, state).await?;
        if let Some(u) = usage {
            state.usage = u;
        }
        Ok(())
    }
}
