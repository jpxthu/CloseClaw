//! Streaming-mode outbound dispatch: pre-flight middleware, incremental
//! rendering, and the post-stream finish pipeline.

use crate::outbound_helpers::{
    dispatch_text, make_outbound_meta, merge_dsl_results, process_single_through_chain,
    send_render_block, CheckpointMeta, StreamContext, StreamState,
};
use crate::outbound_middleware::runner::run_pre_flight_check;
use crate::{Gateway, GatewayError};
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::{LlmState, VerbosityLevel};
use closeclaw_llm::types::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedUsage,
};
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

    /// Post-stream pipeline: select content blocks, run the
    /// Processor Chain (DslParser → OutboundRawLog), and build the
    /// final [`StreamResult`].
    ///
    /// VerbosityFilter is skipped here — it already ran during the
    /// incremental phase via [`ProcessorChain::process_outbound_incremental`].
    async fn finish_streaming_pipeline(
        &self,
        session_blocks: Option<(Vec<ContentBlock>, Option<UnifiedUsage>)>,
        mut state: StreamState,
        channel: &str,
        session_id: &str,
        verbosity_level: VerbosityLevel,
    ) -> Result<StreamResult, GatewayError> {
        let (content_blocks_for_pipeline, usage_override) = match session_blocks {
            Some((blocks, usage)) => (blocks, usage),
            None => (std::mem::take(&mut state.content_blocks), None),
        };

        // Run the outbound Processor Chain (DslParser → OutboundRawLog),
        // skipping VerbosityFilter which already ran during the incremental
        // phase via process_outbound_incremental.
        let meta = make_outbound_meta(&[
            ("channel", channel),
            ("session_id", session_id),
            ("verbosity_level", &verbosity_level.to_string()),
        ]);
        let dsl_result_incremental = state.dsl_instructions.clone();
        let Some(registry) = self.processor_registry.read().unwrap().clone() else {
            let dsl_result = if dsl_result_incremental.is_empty() {
                None
            } else {
                let r = DslParseResult {
                    instructions: dsl_result_incremental,
                };
                serde_json::to_string(&r).ok()
            };
            return Ok(StreamResult {
                content_blocks: content_blocks_for_pipeline,
                usage: usage_override.unwrap_or(state.usage),
                retry_attempts: 0,
                dsl_result,
                // Attached by the production call site from the
                // invocation's returned target.
                call_target: None,
            });
        };
        let input = self.make_outbound_input("", content_blocks_for_pipeline, meta);
        let processed = registry
            .process_outbound_without_verbosity(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))?;

        let dsl_result = merge_dsl_results(&processed.metadata, dsl_result_incremental);

        Ok(StreamResult {
            content_blocks: processed.content_blocks,
            usage: usage_override.unwrap_or(state.usage),
            retry_attempts: 0,
            dsl_result,
            // Attached by the production call site from the invocation's
            // returned target.
            call_target: None,
        })
    }

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
