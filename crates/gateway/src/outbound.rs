//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`](closeclaw_common::im_plugin::IMPlugin) registry.

use super::{Gateway, GatewayError, Message};
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::StreamingOutput;

use closeclaw_common::processor::{DslParseResult, ProcessedMessage};
use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedResponse, UnifiedUsage,
};
use futures::StreamExt;

/// Result of a streaming outbound dispatch.
///
/// Carries the accumulated content blocks (for downstream consumers like
/// `append_response`) and the final token usage reported by the LLM's
/// `MessageEnd` event.
#[derive(Debug, Clone)]
pub struct StreamResult {
    /// All [`ContentBlock`]s produced by the renderer during the stream.
    pub content_blocks: Vec<ContentBlock>,
    /// Token usage statistics from the LLM's `MessageEnd` event.
    pub usage: UnifiedUsage,
}

impl From<UnifiedResponse> for StreamResult {
    /// Convert a non-streaming `UnifiedResponse` into a `StreamResult`.
    ///
    /// Used by the post-LLM completion path (`finish_llm` /
    /// `clear_busy_and_send`) so both streaming and non-streaming
    /// call sites can share the same downstream handling. `finish_reason`
    /// is dropped because `StreamResult` does not carry one.
    fn from(response: UnifiedResponse) -> Self {
        StreamResult {
            content_blocks: response.content_blocks,
            usage: response.usage,
        }
    }
}

impl From<StreamResult> for UnifiedResponse {
    /// Convert a `StreamResult` back into a `UnifiedResponse` for
    /// `ChatSession::append_response`, which only accepts the legacy
    /// shape. `finish_reason` is set to `None` because streaming does
    /// not surface a structured finish reason.
    fn from(result: StreamResult) -> Self {
        UnifiedResponse {
            content_blocks: result.content_blocks,
            usage: result.usage,
            finish_reason: None,
        }
    }
}

/// Per-call context for dispatching a rendered output and persisting its
/// checkpoint. Bundled into a struct to keep the helper's parameter list short.
struct DispatchCtx<'a> {
    plugin: &'a std::sync::Arc<dyn IMPlugin>,
    rendered: &'a RenderedOutput,
    /// Plain-text fallback used when the rendered payload does not carry a
    /// `content.text` field. Typically the processed chain's `content`.
    fallback_text: &'a str,
    session_id: &'a str,
    channel: &'a str,
    chat_id: String,
    /// Optional thread/topic ID for directing the message into a thread.
    thread_id: Option<String>,
}

impl Gateway {
    /// Send an outbound message (agent response) via the registered IM plugin.
    ///
    /// Flow:
    /// 1. Resolve `chat_id` from `session_id` via `SessionManager::get_chat_id`.
    /// 2. Resolve the [`IMPlugin`](super::im::IMPlugin) registered for `channel`
    ///    through `self.plugins`.
    /// 3. Resolve the session's [`VerbosityLevel`] and inject it into chain
    ///    metadata for [`VerbosityFilter`](closeclaw_processor_chain::verbosity_filter::VerbosityFilter).
    /// 4. Run the full outbound processor chain (VerbosityFilter → DslParser →
    ///    OutboundRawLog) via `process_or_bypass`.
    /// 5. Extract `dsl_result` from `processed.metadata["dsl_result"]` (stored
    ///    as a JSON-encoded string by the DSL processor).
    /// 6. Call `plugin.render(blocks, dsl_result)` to obtain a
    ///    [`RenderedOutput`](closeclaw_common::im_plugin::RenderedOutput); fall back to a
    ///    single `ContentBlock::Text` block when `content_blocks` is empty.
    /// 7. Dispatch by `msg_type` (`"text"` / `"interactive"`) through
    ///    `plugin.send`. Any other type is an [`GatewayError::OutboundError`].
    /// 8. After each successful send, trigger checkpoint persistence.
    /// 9. `thread_id` is resolved via `session_manager.get_thread_id` and
    ///    passed to `plugin.send`.
    pub async fn send_outbound(
        &self,
        session_id: &str,
        channel: &str,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
    ) -> Result<(), GatewayError> {
        // 1. Resolve chat_id and plugin.
        let chat_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .ok_or(GatewayError::MissingSessionId)?;
        let plugin = self
            .get_plugin(channel)
            .await
            .ok_or_else(|| GatewayError::UnknownChannel(channel.to_string()))?;

        // 2. Resolve verbosity level and inject into chain metadata.
        let verbosity_level = if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            cs.read().await.verbosity_level()
        } else {
            VerbosityLevel::default()
        };

        // 3. Processor chain (VerbosityFilter → DslParser → OutboundRawLog).
        let processed = self
            .process_or_bypass(
                raw_output,
                content_blocks,
                channel,
                session_id,
                verbosity_level,
            )
            .await?;
        if processed.content_blocks.is_empty() {
            return Ok(());
        }

        let blocks = &processed.content_blocks;

        // 6. Extract dsl_result (serialized as a JSON string by DslParser).
        let dsl_result: Option<DslParseResult> = processed
            .metadata
            .get("dsl_result")
            .and_then(|s| serde_json::from_str(s).ok());

        // 7. Render via the plugin.
        let rendered = plugin.render(blocks, dsl_result.as_ref());

        // 8. Resolve thread_id from session checkpoint.
        let thread_id = self.session_manager.get_thread_id(session_id).await;

        // 9. Dispatch by msg_type and persist checkpoint on success.
        let fallback_text = blocks
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .unwrap_or("");
        self.dispatch_and_persist(DispatchCtx {
            plugin: &plugin,
            rendered: &rendered,
            fallback_text,
            session_id,
            channel,
            chat_id,
            thread_id,
        })
        .await
    }

    /// Dispatch a rendered output to its destination plugin and persist the
    /// outbound checkpoint. `msg_type` drives the dispatch:
    /// - `"text"`: extract text from `rendered.payload`, build a [`Message`],
    ///   call `plugin.send`.
    /// - `"interactive"`: call `plugin.send` directly, build a [`Message`]
    ///   from the serialized payload for checkpointing.
    /// - any other: return [`GatewayError::OutboundError`].
    ///
    /// Before sending, the rendered output is passed through the registered
    /// outbound middleware chain (see [`OutboundMiddleware`]).
    async fn dispatch_and_persist(&self, ctx: DispatchCtx<'_>) -> Result<(), GatewayError> {
        // Run outbound middleware chain (render → middleware → send).
        let middlewares = self.get_outbound_middlewares().await;
        let rendered = if middlewares.is_empty() {
            ctx.rendered.clone()
        } else {
            run_middleware_chain(&middlewares, ctx.rendered.clone())
                .await
                .map_err(|e| GatewayError::OutboundError(e.to_string()))?
        };
        match rendered.msg_type.as_str() {
            "text" => {
                let text = rendered
                    .payload
                    .get("content")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(ctx.fallback_text)
                    .to_string();
                let msg = Self::make_outbound_msg(ctx.channel, ctx.chat_id.clone(), text);
                ctx.plugin
                    .send(&rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                self.persist_outbound_checkpoint(ctx.session_id, &msg).await;
                Ok(())
            }
            "interactive" => {
                let payload_str =
                    serde_json::to_string(&rendered.payload).unwrap_or_else(|_| "{}".to_string());
                ctx.plugin
                    .send(&rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                let msg = Self::make_outbound_msg(ctx.channel, ctx.chat_id, payload_str);
                self.persist_outbound_checkpoint(ctx.session_id, &msg).await;
                Ok(())
            }
            _ => Err(GatewayError::OutboundError(format!(
                "unknown msg_type: {}",
                rendered.msg_type
            ))),
        }
    }

    /// Run the outbound processor chain if configured, otherwise bypass with
    /// a synthetic [`ProcessedMessage`] wrapping the raw input.
    async fn process_or_bypass(
        &self,
        _raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
        session_id: &str,
        verbosity_level: VerbosityLevel,
    ) -> Result<ProcessedMessage, GatewayError> {
        let registry = self.processor_registry.read().unwrap().clone();
        let Some(registry) = registry else {
            let blocks = if content_blocks.is_empty() {
                vec![ContentBlock::Text(_raw_output.to_string())]
            } else {
                content_blocks
            };
            return Ok(ProcessedMessage {
                content_blocks: blocks,
                metadata: std::collections::HashMap::new(),
            });
        };
        let mut meta = std::collections::HashMap::new();
        meta.insert("channel".to_string(), channel.to_string());
        meta.insert("session_id".to_string(), session_id.to_string());
        meta.insert("verbosity_level".to_string(), verbosity_level.to_string());
        let input = ProcessedMessage {
            content_blocks,
            metadata: meta,
        };
        registry
            .process_outbound(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
    }

    /// Build a [`Message`] for checkpoint persistence from outbound fields.
    fn make_outbound_msg(channel: &str, to: String, content: String) -> Message {
        Message {
            id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
            from: "agent".to_string(),
            to,
            content,
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
            thread_id: None,
        }
    }

    /// Persist outbound message to checkpoint if checkpoint_manager is configured.
    async fn persist_outbound_checkpoint(&self, session_id: &str, msg: &Message) {
        let Some(ref cm) = self.checkpoint_manager else {
            return;
        };
        let checkpoint = match cm.load(session_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) => {
                closeclaw_session::persistence::SessionCheckpoint::new(session_id.to_string())
            }
            Err(e) => {
                tracing::warn!(session_id, "failed to load checkpoint: {}", e);
                return;
            }
        };
        let mut pending = closeclaw_session::persistence::PendingMessage::with_role(
            msg.id.clone(),
            msg.content.clone(),
            "assistant".to_string(),
        );
        pending.mark_sent();
        let mut cp = checkpoint.add_pending_message(pending);
        // Sync per-session append-section list from ConversationSession
        // (issue #860: archived session restore preserves append content).
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            let cs = cs.read().await;
            cp.system_appends = cs.user_system_appends().to_vec();
        }
        cp.touch();
        if let Err(e) = cm.save(cp).await {
            tracing::warn!(session_id, "failed to save checkpoint: {}", e);
        }
    }

    /// Send an outbound message to a specific chat via the registered IM plugin.
    ///
    /// This is a lightweight variant of [`send_outbound`](Self::send_outbound) that
    /// does not require a `session_id` — it takes a `chat_id` directly. Useful for
    /// system messages (e.g. busy replies) that have no associated session.
    ///
    /// Flow:
    /// 1. Resolve the [`IMPlugin`](super::im::IMPlugin) for `channel`.
    /// 2. Run the full outbound processor chain (VerbosityFilter → DslParser →
    ///    OutboundRawLog) via `process_or_bypass` with default
    ///    [`VerbosityLevel::Full`].
    /// 3. Render via `plugin.render` and dispatch via `plugin.send`.
    pub async fn send_outbound_to_chat(
        &self,
        chat_id: &str,
        channel: &str,
        raw_output: &str,
    ) -> Result<(), GatewayError> {
        let plugin = self
            .get_plugin(channel)
            .await
            .ok_or_else(|| GatewayError::UnknownChannel(channel.to_string()))?;

        // Processor chain (VerbosityFilter → DslParser → OutboundRawLog).
        let blocks = vec![ContentBlock::Text(raw_output.to_string())];
        let processed = self
            .process_or_bypass(raw_output, blocks, channel, "", VerbosityLevel::default())
            .await?;
        if processed.content_blocks.is_empty() {
            return Ok(());
        }

        // Extract dsl_result stored by the DSL processor.
        let dsl_result: Option<DslParseResult> = processed
            .metadata
            .get("dsl_result")
            .and_then(|s| serde_json::from_str(s).ok());

        // Render via the plugin.
        let mut rendered = plugin.render(&processed.content_blocks, dsl_result.as_ref());

        // Run outbound middleware chain (render → middleware → send).
        let middlewares = self.get_outbound_middlewares().await;
        if !middlewares.is_empty() {
            rendered = run_middleware_chain(&middlewares, rendered)
                .await
                .map_err(|e| GatewayError::OutboundError(e.to_string()))?;
        }

        // Dispatch via plugin.send.
        plugin.send(&rendered, chat_id, None).await?;
        Ok(())
    }

    /// Send a simplified outbound message, skipping the full processor chain
    /// and middleware. Used for non-text message rejection replies where the
    /// design doc specifies a short path: log → render → send.
    pub async fn send_outbound_simplified(
        &self,
        chat_id: &str,
        channel: &str,
        raw_output: &str,
    ) -> Result<(), GatewayError> {
        let plugin = self
            .get_plugin(channel)
            .await
            .ok_or_else(|| GatewayError::UnknownChannel(channel.to_string()))?;
        let blocks = vec![ContentBlock::Text(raw_output.to_string())];

        // Log before render (design doc: 经日志→渲染→发送).
        tracing::info!(
            chat_id,
            channel,
            content = %raw_output,
            "simplified outbound"
        );

        // Render without DSL result — skips Verbosity/DslParser.
        let rendered = plugin.render(&blocks, None);

        // Send directly — no outbound middleware chain.
        plugin.send(&rendered, chat_id, None).await?;
        Ok(())
    }

    /// Send a streaming LLM response via the registered IM plugin.
    ///
    /// Drives a [`DefaultStreamingRenderer`] over the [`StreamEvent`] stream,
    /// dispatching incremental output to `plugin` as it becomes available:
    /// - Text delta → line buffer → complete lines → `plugin.send` (text)
    /// - BlockEnd (non-Text) → `plugin.render(&[block], None)` → `plugin.send`
    /// - MessageEnd → flush remaining content → `plugin.send`
    ///
    /// Accumulated `content_blocks` and the LLM-reported `usage` are returned
    /// in a [`StreamResult`]. `thread_id` is resolved from the session
    /// checkpoint and forwarded to all `plugin.send` calls.
    pub async fn send_outbound_streaming<E: std::fmt::Display>(
        &self,
        session_id: &str,
        channel: &str,
        mut stream: impl futures::Stream<Item = Result<StreamEvent, E>> + Unpin,
        plugin: &std::sync::Arc<dyn IMPlugin>,
    ) -> Result<StreamResult, GatewayError> {
        let chat_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .ok_or(GatewayError::MissingSessionId)?;

        // Resolve thread_id from session checkpoint for outbound thread routing.
        let thread_id = self.session_manager.get_thread_id(session_id).await;

        let verbosity_level = if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            cs.read().await.verbosity_level()
        } else {
            VerbosityLevel::default()
        };
        let middlewares = self.get_outbound_middlewares().await;
        let processor_registry = self.processor_registry.read().unwrap().clone();
        let processor_ref: &dyn closeclaw_common::processor::ProcessorChain =
            match processor_registry.as_ref() {
                Some(r) => r.as_ref(),
                None => {
                    // Fallback: no processor chain configured. Use a no-op
                    // passthrough via a local dummy that returns lines unchanged.
                    // This block is unreachable in practice because
                    // process_or_bypass handles the None case, but we need a
                    // reference for the StreamContext lifetime.
                    &NoopProcessorChain
                }
            };
        let mut state = StreamState::new(verbosity_level);
        while let Some(event_result) = stream.next().await {
            let event = event_result.map_err(|e| GatewayError::OutboundError(e.to_string()))?;
            let ctx = StreamContext {
                plugin,
                chat_id: &chat_id,
                thread_id: thread_id.as_deref(),
                middlewares: &middlewares,
                processor_registry: processor_ref,
            };
            self.process_stream_event(&ctx, event, &mut state).await?;
        }
        tracing::debug!(session_id, channel, "streaming outbound complete");

        // Post-stream: run accumulated content_blocks through the three-step
        // outbound pipeline.
        // Real-time per-block verbosity filtering above is kept as an
        // incremental optimization; the final result comes from the pipeline.

        // Processor chain (VerbosityFilter → DslParser → OutboundRawLog).
        let mut processed = self
            .process_or_bypass(
                "",
                state.content_blocks,
                channel,
                session_id,
                verbosity_level,
            )
            .await?;
        state.content_blocks = processed.content_blocks;

        // Merge streaming DslParser results into the batch pipeline output.
        // During streaming, DslParser processed each text line individually and
        // accumulated results in `state.dsl_results`. The batch DslParser runs
        // on already-clean content_blocks (DSL stripped), so it produces an
        // empty result that overwrites the streaming results. Merge them back
        // to avoid losing DSL instructions extracted during streaming.
        if !state.dsl_results.instructions.is_empty() {
            let streaming_json = serde_json::to_string(&state.dsl_results).unwrap_or_default();
            processed
                .metadata
                .insert("dsl_result".to_string(), streaming_json);
        }

        Ok(StreamState::into_result(state))
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
                self.handle_block_end(ctx, event, block_type, state).await?;
            }
            StreamEvent::MessageEnd { usage, .. } => {
                self.handle_message_end(ctx, usage, state).await?;
            }
            StreamEvent::Error { message } => {
                let partial_content = std::mem::take(&mut state.content_blocks);
                let partial_len = partial_content.len();
                tracing::warn!(
                    session_id = ctx.chat_id,
                    error = %message,
                    partial_content_blocks = partial_len,
                    "streaming error with partial content preserved"
                );
                return Err(GatewayError::StreamError {
                    message,
                    partial_content,
                });
            }
            StreamEvent::BlockStart { .. } => {
                ctx.plugin.handle_stream_event(event);
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

    /// Handle a [`StreamEvent::BlockEnd`]: apply per-block verbosity
    /// filtering, send non-text render blocks, and dispatch remaining text.
    async fn handle_block_end(
        &self,
        ctx: &StreamContext<'_>,
        event: StreamEvent,
        block_type: ContentBlockType,
        state: &mut StreamState,
    ) -> Result<(), GatewayError> {
        // Per-block verbosity filtering for non-Text blocks.
        let should_filter = block_type != ContentBlockType::Text
            && match state.verbosity_level {
                VerbosityLevel::Normal => {
                    matches!(block_type, ContentBlockType::Thinking)
                }
                VerbosityLevel::Off => true,
                VerbosityLevel::Full => false,
            };
        if should_filter {
            // Still delegate to plugin so internal state is updated,
            // but discard output (no render, no send, no accumulate).
            ctx.plugin.handle_stream_event(event);
            return Ok(());
        }
        let mut out = ctx.plugin.handle_stream_event(event);
        if block_type != ContentBlockType::Text {
            let render_blocks = std::mem::take(&mut out.render_blocks);
            for block in render_blocks {
                send_render_block(ctx, &block).await?;
                state.content_blocks.push(block);
            }
        }
        dispatch_text(ctx, out, state).await
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

// ---------------------------------------------------------------------------
// Outbound middleware chain
// ---------------------------------------------------------------------------

/// Run a chain of outbound middlewares on a rendered output.
///
/// Processes `rendered` through each middleware in order. If any middleware
/// returns an error, the chain short-circuits and the error is propagated.
async fn run_middleware_chain(
    middlewares: &[std::sync::Arc<dyn closeclaw_common::OutboundMiddleware>],
    rendered: RenderedOutput,
) -> Result<RenderedOutput, closeclaw_common::MiddlewareError> {
    let mut current = rendered;
    for mw in middlewares {
        current = mw.process(&current).await?;
    }
    Ok(current)
}

// ---------------------------------------------------------------------------
// Streaming outbound helpers
// ---------------------------------------------------------------------------

/// Bundles the streaming outbound context passed to `process_stream_event` and
/// its sub-handlers. Keeps parameter counts ≤6 (CONTRIBUTING.md limit).
struct StreamContext<'a> {
    plugin: &'a std::sync::Arc<dyn IMPlugin>,
    chat_id: &'a str,
    thread_id: Option<&'a str>,
    middlewares: &'a [std::sync::Arc<dyn closeclaw_common::OutboundMiddleware>],
    processor_registry: &'a dyn closeclaw_common::processor::ProcessorChain,
}

/// Mutable state carried across stream events in `send_outbound_streaming`.
struct StreamState {
    content_blocks: Vec<ContentBlock>,
    usage: UnifiedUsage,
    /// Verbosity level resolved once per stream; drives per-block filtering.
    verbosity_level: VerbosityLevel,
    /// DSL instructions accumulated during streaming, merged post-stream.
    dsl_results: DslParseResult,
}

impl StreamState {
    fn new(verbosity_level: VerbosityLevel) -> Self {
        Self {
            content_blocks: Vec::new(),
            usage: UnifiedUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            verbosity_level,
            dsl_results: DslParseResult {
                instructions: vec![],
            },
        }
    }

    fn into_result(self) -> StreamResult {
        StreamResult {
            content_blocks: self.content_blocks,
            usage: self.usage,
        }
    }
}

/// Send any text messages from `out` into `state`.
///
/// Each line is processed through the DslParser (zero-overhead passthrough
/// for non-DSL lines), logged to the outbound trace, and sent as clean text.
async fn dispatch_text(
    ctx: &StreamContext<'_>,
    out: StreamingOutput,
    state: &mut StreamState,
) -> Result<(), GatewayError> {
    for text in out.text_messages {
        let (clean_text, dsl_result) = ctx.processor_registry.parse_line_for_dsl(&text);
        state
            .dsl_results
            .instructions
            .extend(dsl_result.instructions);
        tracing::info!(
            chat_id = ctx.chat_id,
            content = %clean_text,
            "streaming outbound text"
        );
        if !clean_text.is_empty() {
            send_text(ctx, &clean_text).await?;
            state.content_blocks.push(ContentBlock::Text(clean_text));
        }
    }
    Ok(())
}

/// Construct a text [`RenderedOutput`] and dispatch via `plugin.send`.
async fn send_text(ctx: &StreamContext<'_>, text: &str) -> Result<(), GatewayError> {
    let rendered = RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!({ "content": { "text": text } }),
    };
    ctx.plugin
        .send(&rendered, ctx.chat_id, ctx.thread_id)
        .await?;
    Ok(())
}

/// Call `plugin.render(&[block], None)`, run outbound middleware, and dispatch via `plugin.send`.
///
/// Logs the rendered content to the outbound trace before sending,
/// ensuring non-text blocks (Thinking/ToolUse/Image/Audio/File) are
/// captured by the Gateway outbound log alongside text blocks.
async fn send_render_block(
    ctx: &StreamContext<'_>,
    block: &ContentBlock,
) -> Result<(), GatewayError> {
    let mut rendered = ctx.plugin.render(std::slice::from_ref(block), None);
    if !ctx.middlewares.is_empty() {
        rendered = run_middleware_chain(ctx.middlewares, rendered)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))?;
    }
    tracing::info!(
        chat_id = ctx.chat_id,
        content = ?rendered.payload,
        msg_type = %rendered.msg_type,
        "streaming outbound render block"
    );
    ctx.plugin
        .send(&rendered, ctx.chat_id, ctx.thread_id)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Verbosity filtering
// ---------------------------------------------------------------------------

/// No-op processor chain fallback when no processor registry is configured.
/// Returns lines unchanged — zero-overhead passthrough.
#[derive(Debug)]
struct NoopProcessorChain;

#[async_trait::async_trait]
impl closeclaw_common::processor::ProcessorChain for NoopProcessorChain {
    async fn process_inbound(
        &self,
        msg: closeclaw_common::im_plugin::NormalizedMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        Ok(closeclaw_common::processor::ProcessedMessage {
            content_blocks: vec![closeclaw_common::processor::ContentBlock::Text(msg.content)],
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn process_outbound(
        &self,
        msg: closeclaw_common::processor::ProcessedMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        Ok(msg)
    }
}

/// Filter content blocks based on the session's verbosity level.
///
/// - [`VerbosityLevel::Full`]: no filtering, all blocks are kept.
/// - [`VerbosityLevel::Normal`]: remove [`ContentBlock::Thinking`] blocks.
/// - [`VerbosityLevel::Off`]: only keep [`ContentBlock::Text`] blocks.
#[allow(dead_code)]
pub(crate) fn filter_by_verbosity(
    blocks: Vec<ContentBlock>,
    level: VerbosityLevel,
) -> Vec<ContentBlock> {
    match level {
        VerbosityLevel::Full => blocks,
        VerbosityLevel::Normal => blocks
            .into_iter()
            .filter(|b| !matches!(b, ContentBlock::Thinking { .. }))
            .collect(),
        VerbosityLevel::Off => blocks
            .into_iter()
            .filter(|b| matches!(b, ContentBlock::Text(_)))
            .collect(),
    }
}
