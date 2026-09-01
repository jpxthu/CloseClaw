//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`](closeclaw_common::im_plugin::IMPlugin) registry.

use super::{Gateway, GatewayError, Message};
use crate::outbound_helpers::{
    dispatch_text, log_middleware_rejection, make_outbound_meta, merge_dsl_results,
    notify_batch_send_failure, process_single_through_chain, send_render_block, StreamContext,
    StreamState,
};
use closeclaw_common::im_plugin::{IMPlugin, RenderedOutput};
use closeclaw_common::MiddlewareContext;
use closeclaw_processor_chain::run_middleware_chain;
use std::sync::Arc;

use closeclaw_common::processor::{DslParseResult, ProcessedMessage};
use closeclaw_common::LlmState;
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
    /// Number of retry attempts made before the LLM call succeeded.
    pub retry_attempts: u32,
    /// DSL instruction result extracted from processor chain metadata.
    pub dsl_result: Option<String>,
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
            retry_attempts: response.retry_attempts,
            dsl_result: None,
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
            retry_attempts: result.retry_attempts,
        }
    }
}

/// Bundled context for streaming outbound dispatch.
///
/// Groups trace metadata and optional session-assembled content that
/// every streaming outbound helper needs, keeping individual parameter
/// lists within the project's 6-parameter hard limit.
#[derive(Debug, Clone, Default)]
pub struct OutboundMeta {
    /// Inbound trace ID for debug-log event correlation.
    pub trace_id: Option<String>,
    /// Inbound session key for debug-log event correlation.
    pub session_key: Option<String>,
    /// Root span ID for debug-log child span derivation.
    pub span_id: Option<String>,
    /// Session-assembled content blocks (used by `send_outbound_streaming_assembled`).
    pub session_content_blocks: Vec<ContentBlock>,
    /// Session-assembled usage override.
    pub session_usage: Option<UnifiedUsage>,
}

/// Outcome of a single outbound dispatch.
///
/// Distinguishes between "message delivered" and "delivery failed but user
/// notified" so callers like the drain loop can keep `delivered` counts honest
/// without double-notifying or retrying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Original message delivered successfully.
    Sent,
    /// Original message delivery failed; a failure notification was sent to
    /// the user via the simplified path. The message was NOT delivered.
    Notified,
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
    /// Outbound directed reference for定向回复.
    reply_ref: Option<String>,
    /// DSL result string from the processor chain (JSON serialized).
    dsl_result: Option<String>,
    /// Serialized content blocks (JSON) for checkpoint persistence.
    content_blocks: Option<String>,
    /// Inbound trace_id for debug log event correlation.
    trace_id: Option<String>,
    /// Inbound session_key for debug log event correlation.
    session_key: Option<String>,
}

impl Gateway {
    /// Send an outbound message (agent response) via the registered IM plugin.
    ///
    /// Flow: resolve chat_id + plugin → resolve VerbosityLevel → run full batch
    /// outbound chain (VerbosityFilter → DslParser → OutboundRawLog) → render →
    /// dispatch by msg_type → persist checkpoint.
    ///
    /// Streaming mode also runs VerbosityFilter — during the incremental phase
    /// at block boundaries — and skips it in the finish phase (see
    /// [`finish_streaming_pipeline`]).
    pub async fn send_outbound(
        &self,
        session_id: &str,
        channel: &str,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        trace_id: Option<String>,
        session_key: Option<String>,
    ) -> Result<SendOutcome, GatewayError> {
        // 1. Resolve chat_id and plugin.
        let chat_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .ok_or(GatewayError::MissingSessionId)?;
        let Some(plugin) = self.get_plugin(channel).await else {
            return self
                .fallback_to_plain_text(channel, raw_output)
                .await
                .map(|()| SendOutcome::Sent);
        };

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

        // 3. Full batch outbound chain (VerbosityFilter → DslParser → OutboundRawLog).
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
            return Ok(SendOutcome::Sent);
        }

        let blocks = &processed.content_blocks;
        let dsl_result: Option<DslParseResult> = processed
            .metadata
            .get("dsl_result")
            .and_then(|s| serde_json::from_str(s).ok());
        let render_start = std::time::Instant::now();
        let rendered = plugin.render(blocks, dsl_result.as_ref());
        if channel == "feishu" {
            let render_duration_ms = render_start.elapsed().as_millis() as u64;
            crate::outbound_helpers::emit_feishu_render_event(
                self,
                trace_id.as_deref().unwrap_or(""),
                session_key.as_deref(),
                channel,
                render_duration_ms,
                None, // outbound render event, no parent context needed
            );
        }
        let thread_id = self.session_manager.get_thread_id(session_id).await;
        let reply_ref = self.session_manager.get_reply_ref(session_id).await;
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
            reply_ref,
            dsl_result: processed.metadata.get("dsl_result").cloned(),
            content_blocks: serde_json::to_string(&processed.content_blocks).ok(),
            trace_id,
            session_key,
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
    async fn dispatch_and_persist(
        &self,
        ctx: DispatchCtx<'_>,
    ) -> Result<SendOutcome, GatewayError> {
        // Run outbound middleware chain (render → middleware → send).
        let middlewares = self.get_outbound_middlewares().await;
        if !middlewares.is_empty() {
            let mctx = Self::make_middleware_ctx(ctx.session_id, ctx.channel, &ctx.chat_id);
            if let Err(e) = run_middleware_chain(&middlewares, &mctx, ctx.rendered).await {
                return log_middleware_rejection(self, e, &ctx.chat_id, ctx.channel)
                    .await
                    .map(|()| SendOutcome::Sent);
            }
        }
        // Send via plugin — on failure, notify user and skip outbound history.
        let send_start = std::time::Instant::now();
        let send_result = ctx
            .plugin
            .send(
                ctx.rendered,
                &ctx.chat_id,
                ctx.thread_id.as_deref(),
                ctx.reply_ref.as_deref(),
            )
            .await;
        if ctx.channel == "feishu" {
            let send_duration_ms = send_start.elapsed().as_millis() as u64;
            crate::outbound_helpers::emit_feishu_send_event(
                self,
                ctx.trace_id.as_deref().unwrap_or(""),
                ctx.session_key.as_deref(),
                ctx.channel,
                &ctx.chat_id,
                send_duration_ms,
                None, // outbound send event, no parent context needed
            );
        }
        if let Err(e) = send_result {
            notify_batch_send_failure(self, ctx.channel, &ctx.chat_id, e).await;
            return Ok(SendOutcome::Notified);
        }
        // Extract content for checkpoint based on msg_type.
        let content = crate::outbound_helpers::extract_content_for_checkpoint(
            ctx.rendered,
            ctx.fallback_text,
        )?;
        let msg = Self::make_outbound_msg(
            ctx.channel,
            ctx.chat_id.clone(),
            content,
            Some(ctx.channel.to_string()),
            ctx.dsl_result.clone(),
            ctx.content_blocks.clone(),
        );
        self.persist_outbound_checkpoint(ctx.session_id, &msg, true)
            .await;
        self.emit_send_completed_log(
            ctx.session_id,
            ctx.channel,
            &ctx.chat_id,
            ctx.trace_id.as_deref(),
            ctx.session_key.as_deref(),
            None, // outbound send event, no parent context needed
        );
        Ok(SendOutcome::Sent)
    }

    /// Emit a unified `send.completed` debug log event.
    ///
    /// Extracted from the text/interactive branches in
    /// [`dispatch_and_persist`] to eliminate duplicated emit code.
    /// When `trace_id` is `None`, the emit is skipped.
    fn emit_send_completed_log(
        &self,
        _session_id: &str,
        channel: &str,
        peer_id: &str,
        trace_id: Option<&str>,
        session_key: Option<&str>,
        parent: Option<&closeclaw_debug_log::TraceContext>,
    ) {
        let Some(tid) = trace_id else {
            return;
        };
        let guard = self.debug_log.read().unwrap_or_else(|e| e.into_inner());
        crate::debug_log_emitter::emit_debug_event(crate::debug_log_emitter::EmitEventParams {
            ctx: crate::debug_log_emitter::DebugLogContext::new(guard.as_ref(), tid, session_key),
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "gateway",
            event_type: "send.completed",
            payload: serde_json::json!({
                "channel": channel,
                "peer_id": peer_id,
            }),
            parent,
        });
    }

    /// Run only the outbound raw-log processor, bypassing the full chain.
    ///
    /// Used by [`send_outbound_simplified`] for non-text message rejection
    /// replies where the design doc requires log → render → send without
    /// VerbosityFilter / DslParser / middleware.
    pub(crate) async fn process_outbound_raw_log_only(
        &self,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
    ) -> Result<ProcessedMessage, GatewayError> {
        let meta = make_outbound_meta(&[("channel", channel)]);
        let input = self.make_outbound_input(raw_output, content_blocks, meta);
        let Some(registry) = self.processor_registry.read().unwrap().clone() else {
            return Ok(input);
        };
        registry
            .process_outbound_raw_log_only(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
    }

    /// Run the outbound processor chain if configured, otherwise bypass.
    pub(crate) async fn process_or_bypass(
        &self,
        _raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
        session_id: &str,
        verbosity_level: VerbosityLevel,
    ) -> Result<ProcessedMessage, GatewayError> {
        let meta = make_outbound_meta(&[
            ("channel", channel),
            ("session_id", session_id),
            ("verbosity_level", &verbosity_level.to_string()),
        ]);
        let input = self.make_outbound_input(_raw_output, content_blocks, meta);
        let Some(registry) = self.processor_registry.read().unwrap().clone() else {
            return Ok(input);
        };
        registry
            .process_outbound(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
    }

    /// Build a [`ProcessedMessage`] from raw output.
    fn make_outbound_input(
        &self,
        _raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        metadata: std::collections::HashMap<String, String>,
    ) -> ProcessedMessage {
        let blocks = if content_blocks.is_empty() {
            vec![ContentBlock::Text(_raw_output.to_string())]
        } else {
            content_blocks
        };
        ProcessedMessage {
            content_blocks: blocks,
            metadata,
        }
    }

    /// Fallback to plain-text output when no IM plugin is registered for
    /// the target channel. Logs a warning, records the raw text to the
    /// outbound log (via `process_outbound_raw_log_only`), and returns `Ok(())`
    /// so the caller does not fail.
    pub(crate) async fn fallback_to_plain_text(
        &self,
        channel: &str,
        raw_output: &str,
    ) -> Result<(), GatewayError> {
        tracing::warn!(
            channel,
            "no IM plugin registered, falling back to plain-text log"
        );
        let blocks = vec![ContentBlock::Text(raw_output.to_string())];
        self.process_outbound_raw_log_only(raw_output, blocks, channel)
            .await?;
        Ok(())
    }

    /// Fallback to plain-text send when render/send fails.
    pub(crate) async fn send_as_plain_text(
        &self,
        plugin: &Arc<dyn IMPlugin>,
        raw_output: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), GatewayError> {
        tracing::warn!(
            chat_id,
            "render/send failed, falling back to plain-text send"
        );
        let rendered = RenderedOutput {
            msg_type: "text".to_string(),
            payload: serde_json::json!({ "content": { "text": raw_output } }),
        };
        plugin.send(&rendered, chat_id, thread_id, None).await?;
        Ok(())
    }

    /// Build a [`Message`] for checkpoint persistence from outbound fields.
    fn make_outbound_msg(
        channel: &str,
        to: String,
        content: String,
        platform: Option<String>,
        dsl_result: Option<String>,
        content_blocks: Option<String>,
    ) -> Message {
        Message {
            id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
            from: "agent".to_string(),
            to,
            content,
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
            thread_id: None,
            reply_ref: None,
            platform,
            dsl_result,
            content_blocks,
        }
    }

    /// Persist outbound message to checkpoint if checkpoint_manager is configured.
    ///
    /// When `mark_sent` is `true`, the pending message is marked as sent
    /// (checkpoint saved after successful delivery). When `false`, the
    /// pending message is persisted without the sent flag, serving as a
    /// pre-send checkpoint so recovery can detect the pending operation.
    pub(crate) async fn persist_outbound_checkpoint(
        &self,
        session_id: &str,
        msg: &Message,
        mark_sent: bool,
    ) {
        let cm = self.checkpoint_manager.read().unwrap().clone();
        let Some(cm) = cm else {
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
        pending.target_channel = msg.channel.clone();
        pending.platform = msg.platform.clone();
        pending.dsl_result = msg.dsl_result.clone();
        pending.content_blocks = msg.content_blocks.clone();
        if mark_sent {
            pending.mark_sent();
        }
        let mut cp = checkpoint.add_outbound_pending(pending);
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
        cp.last_message_at = Some(chrono::Utc::now());
        if let Err(e) = cm.save(cp).await {
            tracing::warn!(session_id, "failed to save checkpoint: {}", e);
        }
    }

    /// Send a streaming LLM response via the registered IM plugin.
    ///
    /// Delegates to [`send_outbound_streaming_inner`] for core logic.
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
        let chat_id = self
            .session_manager
            .get_chat_id(session_id)
            .await
            .ok_or(GatewayError::MissingSessionId)?;

        // Resolve thread_id from session checkpoint for outbound thread routing.
        let thread_id = self.session_manager.get_thread_id(session_id).await;
        let reply_ref = self.session_manager.get_reply_ref(session_id).await;

        let verbosity_level = if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            cs.read().await.verbosity_level()
        } else {
            VerbosityLevel::default()
        };

        // Pre-flight middleware check: execute once before the stream loop
        // instead of per-chunk. This avoids middleware overhead on every
        // incremental update and aligns with the design doc requirement
        // that streaming pre-flight runs exactly once using session metadata.
        let middlewares = self.get_outbound_middlewares().await;
        if !middlewares.is_empty() {
            let mctx = Self::make_middleware_ctx(session_id, channel, &chat_id);
            if let Err(e) =
                closeclaw_processor_chain::run_pre_flight_check(&middlewares, &mctx).await
            {
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
            thread_id: thread_id.as_deref(),
            reply_ref: reply_ref.as_deref(),
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
        let msg = Self::make_outbound_msg(
            channel,
            chat_id,
            text,
            Some(channel.to_string()),
            result.dsl_result.clone(),
            Some(content_blocks_json),
        );
        self.persist_outbound_checkpoint(session_id, &msg, true)
            .await;

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
                // Flush any in-progress text from the renderer so partial
                // content from incomplete blocks is not lost.
                let flush_out = ctx.plugin.flush_stream();
                for text in flush_out.text_messages {
                    if !text.is_empty() {
                        state.content_blocks.push(ContentBlock::Text(text));
                    }
                }
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

    pub(crate) fn make_middleware_ctx(
        session_id: &str,
        channel: &str,
        chat_id: &str,
    ) -> MiddlewareContext {
        MiddlewareContext {
            session_id: session_id.to_string(),
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
        }
    }
}
