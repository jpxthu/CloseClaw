//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`](closeclaw_common::im_plugin::IMPlugin) registry.

use super::{Gateway, GatewayError, Message};
use crate::outbound_helpers::dispatch_text;
use crate::outbound_helpers::filter_by_verbosity;
use crate::outbound_helpers::log_middleware_rejection;
use crate::outbound_helpers::send_render_block;
use crate::outbound_helpers::StreamContext;
use crate::outbound_helpers::StreamState;
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::MiddlewareContext;
use closeclaw_debug_log::{LogEvent, LogLevel, TraceContext};
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
    /// DSL result string from the processor chain (JSON serialized).
    dsl_result: Option<String>,
    /// Serialized content blocks (JSON) for checkpoint persistence.
    content_blocks: Option<String>,
}

impl Gateway {
    /// Send an outbound message (agent response) via the registered IM plugin.
    ///
    /// Flow: resolve chat_id + plugin → resolve VerbosityLevel → run processor
    /// chain (VerbosityFilter → DslParser → OutboundRawLog) → render → dispatch
    /// by msg_type → persist checkpoint.
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
        let Some(plugin) = self.get_plugin(channel).await else {
            return self.fallback_to_plain_text(channel, raw_output).await;
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
        // On render/send failure, fall back to plain-text send.
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
            chat_id: chat_id.clone(),
            thread_id: thread_id.clone(),
            dsl_result: processed.metadata.get("dsl_result").cloned(),
            content_blocks: serde_json::to_string(&processed.content_blocks).ok(),
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
        if !middlewares.is_empty() {
            let mctx = Self::make_middleware_ctx(ctx.session_id, ctx.channel, &ctx.chat_id);
            if let Err(e) = run_middleware_chain(&middlewares, &mctx, ctx.rendered).await {
                return log_middleware_rejection(e, ctx.session_id);
            }
        }
        match ctx.rendered.msg_type.as_str() {
            "text" => {
                let text = ctx
                    .rendered
                    .payload
                    .get("content")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(ctx.fallback_text)
                    .to_string();
                let msg = Self::make_outbound_msg(
                    ctx.channel,
                    ctx.chat_id.clone(),
                    text,
                    Some(ctx.channel.to_string()),
                    ctx.dsl_result.clone(),
                    ctx.content_blocks.clone(),
                );
                // Pre-send checkpoint: persist pending before delivery so
                // recovery can detect the pending operation on crash.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, false)
                    .await;
                ctx.plugin
                    .send(ctx.rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                // Post-send checkpoint: mark as sent after successful delivery.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, true)
                    .await;
                // Debug log: send.completed
                self.emit_send_completed_log(ctx.session_id, ctx.channel, &ctx.chat_id)
                    .await;
                Ok(())
            }
            "interactive" => {
                let payload_str = serde_json::to_string(&ctx.rendered.payload)
                    .unwrap_or_else(|_| "{}".to_string());
                let msg = Self::make_outbound_msg(
                    ctx.channel,
                    ctx.chat_id.clone(),
                    payload_str,
                    Some(ctx.channel.to_string()),
                    ctx.dsl_result.clone(),
                    ctx.content_blocks.clone(),
                );
                // Pre-send checkpoint: persist pending before delivery so
                // recovery can detect the pending operation on crash.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, false)
                    .await;
                ctx.plugin
                    .send(ctx.rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                // Post-send checkpoint: mark as sent after successful delivery.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, true)
                    .await;
                // Debug log: send.completed
                self.emit_send_completed_log(ctx.session_id, ctx.channel, &ctx.chat_id)
                    .await;
                Ok(())
            }
            _ => Err(GatewayError::OutboundError(format!(
                "unknown msg_type: {}",
                ctx.rendered.msg_type
            ))),
        }
    }

    /// Run only the outbound raw-log processor, bypassing the full chain.
    ///
    /// Used by [`send_outbound_simplified`] for non-text message rejection
    /// replies where the design doc requires log → render → send without
    /// VerbosityFilter / DslParser / middleware.
    async fn process_outbound_raw_log_only(
        &self,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
    ) -> Result<ProcessedMessage, GatewayError> {
        let meta = Self::make_outbound_meta(&[("channel", channel)]);
        let input = self.make_outbound_input(raw_output, content_blocks, meta);
        let Some(registry) = self.processor_registry.read().unwrap().clone() else {
            return Ok(input);
        };
        registry
            .process_outbound_raw_log_only(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
    }

    /// Run the outbound chain with VerbosityFilter skipped.
    ///
    /// Used by the streaming pipeline finish phase where verbosity filtering
    /// is handled inline during the stream (not in the post-stream chain).
    async fn process_outbound_skip_verbosity(
        &self,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
        session_id: &str,
    ) -> Result<ProcessedMessage, GatewayError> {
        let meta = Self::make_outbound_meta(&[("channel", channel), ("session_id", session_id)]);
        let input = self.make_outbound_input(raw_output, content_blocks, meta);
        let Some(registry) = self.processor_registry.read().unwrap().clone() else {
            return Ok(input);
        };
        registry
            .process_outbound_skip_verbosity(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
    }

    /// Run the outbound processor chain if configured, otherwise bypass.
    async fn process_or_bypass(
        &self,
        _raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        channel: &str,
        session_id: &str,
        verbosity_level: VerbosityLevel,
    ) -> Result<ProcessedMessage, GatewayError> {
        let meta = Self::make_outbound_meta(&[
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
        ProcessedMessage {
            content_blocks,
            metadata,
        }
    }

    fn make_outbound_meta(entries: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Fallback to plain-text output when no IM plugin is registered for
    /// the target channel. Logs a warning, records the raw text to the
    /// outbound log (via `process_outbound_raw_log_only`), and returns `Ok(())`
    /// so the caller does not fail.
    async fn fallback_to_plain_text(
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
    async fn send_as_plain_text(
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
        plugin.send(&rendered, chat_id, thread_id).await?;
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

    /// Emit a `send.completed` debug log event after a successful outbound send.
    async fn emit_send_completed_log(&self, session_id: &str, channel: &str, peer_id: &str) {
        let guard = match self.debug_log.read() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(ref debug_log) = *guard else {
            return;
        };
        // Generate a fresh trace_id for this outbound event; correlation
        // with inbound trace_id requires deeper plumbing (future work).
        let trace_id = format!(
            "out-{}-{}",
            session_id,
            chrono::Utc::now().timestamp_millis()
        );
        let ctx = TraceContext::new_root(trace_id);
        let event = LogEvent::new(
            &ctx,
            None,
            LogLevel::Info,
            "gateway",
            "send.completed",
            serde_json::json!({
                "channel": channel,
                "peer_id": peer_id,
            }),
        );
        let debug_log = debug_log.clone();
        tokio::spawn(async move {
            debug_log.log(event).await;
        });
    }

    /// Lightweight outbound to a specific chat (no session_id required).
    /// Useful for system messages like busy replies.
    pub async fn send_outbound_to_chat(
        &self,
        chat_id: &str,
        channel: &str,
        raw_output: &str,
    ) -> Result<(), GatewayError> {
        let Some(plugin) = self.get_plugin(channel).await else {
            return self.fallback_to_plain_text(channel, raw_output).await;
        };

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
        let rendered = plugin.render(&processed.content_blocks, dsl_result.as_ref());

        // Run outbound middleware chain (render → middleware → send).
        let middlewares = self.get_outbound_middlewares().await;
        if !middlewares.is_empty() {
            let mctx = Self::make_middleware_ctx("", channel, chat_id);
            if let Err(e) = run_middleware_chain(&middlewares, &mctx, &rendered).await {
                return log_middleware_rejection(e, chat_id);
            }
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
        let Some(plugin) = self.get_plugin(channel).await else {
            return self.fallback_to_plain_text(channel, raw_output).await;
        };
        let blocks = vec![ContentBlock::Text(raw_output.to_string())];

        // Run only the outbound raw-log processor (skip Verbosity/DslParser).
        let processed = self
            .process_outbound_raw_log_only(raw_output, blocks.clone(), channel)
            .await?;
        if processed.content_blocks.is_empty() {
            return Ok(());
        }

        // Render without DSL result — skips Verbosity/DslParser.
        let rendered = plugin.render(&processed.content_blocks, None);

        // Send directly — no outbound middleware chain.
        // On render/send failure, fall back to plain-text send.
        if plugin.send(&rendered, chat_id, None).await.is_err() {
            self.send_as_plain_text(&plugin, raw_output, chat_id, None)
                .await
        } else {
            Ok(())
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
    ) -> Result<StreamResult, GatewayError> {
        self.send_outbound_streaming_inner(session_id, channel, stream, plugin, None)
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
        session_content_blocks: Vec<ContentBlock>,
        session_usage: Option<UnifiedUsage>,
    ) -> Result<StreamResult, GatewayError> {
        self.send_outbound_streaming_inner(
            session_id,
            channel,
            stream,
            plugin,
            Some((session_content_blocks, session_usage)),
        )
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
        session_blocks: Option<(Vec<ContentBlock>, Option<UnifiedUsage>)>,
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
        let mut state = StreamState::new(verbosity_level);
        let mut first_event_received = false;
        let timeout_duration = std::time::Duration::from_millis(200);
        let ctx = StreamContext {
            plugin,
            session_id,
            channel,
            chat_id: &chat_id,
            thread_id: thread_id.as_deref(),
            middlewares: &middlewares,
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

        self.finish_streaming_pipeline(session_blocks, state, channel, session_id, verbosity_level)
            .await
    }

    /// Post-stream pipeline: select content blocks, run processor chain
    /// (skipping VerbosityFilter — handled inline during the stream),
    /// merge DSL results, and build the final [`StreamResult`].
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

        // Skip VerbosityFilter in the processor chain — the design doc
        // requires: "收尾阶段不重跑 VerbosityFilter". VerbosityFilter is
        // applied inline during BlockEnd streaming events. However, the
        // final content_blocks must still be filtered for downstream
        // consumers, so we apply VerbosityFilter directly here.
        let processed = self
            .process_outbound_skip_verbosity("", content_blocks_for_pipeline, channel, session_id)
            .await?;

        let filtered_blocks = filter_by_verbosity(processed.content_blocks, verbosity_level);

        Ok(StreamResult {
            content_blocks: filtered_blocks,
            usage: usage_override.unwrap_or(state.usage),
            retry_attempts: 0,
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
                // Thinking indicator: send stop signal before verbosity filtering.
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
    /// and dispatch remaining text. Verbosity filtering is delegated to
    /// the post-stream Processor Chain in [`finish_streaming_pipeline`].
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
                // Filter non-Text blocks through VerbosityFilter for
                // real-time send. VerbosityLevel::Off suppresses all
                // non-Text output; VerbosityLevel::Normal suppresses
                // Thinking blocks; VerbosityLevel::Full passes all.
                let filtered = filter_by_verbosity(render_blocks.clone(), state.verbosity_level);
                for block in &filtered {
                    send_render_block(ctx, block).await?;
                }
                // Push ALL original blocks to content_blocks so the
                // post-stream Processor Chain has the full data set.
                state.content_blocks.extend(render_blocks);
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
