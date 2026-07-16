//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`](closeclaw_common::im_plugin::IMPlugin) registry.

use super::{Gateway, GatewayError, Message};
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::StreamingOutput;
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
                // Pre-send checkpoint: persist pending before delivery so
                // recovery can detect the pending operation on crash.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, false)
                    .await;
                ctx.plugin
                    .send(&rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                // Post-send checkpoint: mark as sent after successful delivery.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, true)
                    .await;
                Ok(())
            }
            "interactive" => {
                let payload_str =
                    serde_json::to_string(&rendered.payload).unwrap_or_else(|_| "{}".to_string());
                let msg = Self::make_outbound_msg(ctx.channel, ctx.chat_id.clone(), payload_str);
                // Pre-send checkpoint: persist pending before delivery so
                // recovery can detect the pending operation on crash.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, false)
                    .await;
                ctx.plugin
                    .send(&rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                // Post-send checkpoint: mark as sent after successful delivery.
                self.persist_outbound_checkpoint(ctx.session_id, &msg, true)
                    .await;
                Ok(())
            }
            _ => Err(GatewayError::OutboundError(format!(
                "unknown msg_type: {}",
                rendered.msg_type
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
        let registry = self.processor_registry.read().unwrap().clone();
        let Some(registry) = registry else {
            let blocks = if content_blocks.is_empty() {
                vec![ContentBlock::Text(raw_output.to_string())]
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
        let input = ProcessedMessage {
            content_blocks,
            metadata: meta,
        };
        registry
            .process_outbound_raw_log_only(input)
            .await
            .map_err(|e| GatewayError::OutboundError(e.to_string()))
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

    /// Fallback to plain-text output when no IM plugin is registered for
    /// the target channel. Logs a warning, records the raw text to the
    /// outbound log (via `process_outbound_raw_log_only`), and returns `Ok(())`
    /// so the caller does not fail.
    #[allow(dead_code)] // used by send_outbound* in Step 1.2
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

    /// Fallback to plain-text send when the plugin exists but `render` or
    /// `send` failed. Constructs a plain-text [`RenderedOutput`] and retries
    /// via `plugin.send`. Returns the send result.
    #[allow(dead_code)] // used by send_outbound* in Step 1.2
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
    ///
    /// When `mark_sent` is `true`, the pending message is marked as sent
    /// (checkpoint saved after successful delivery). When `false`, the
    /// pending message is persisted without the sent flag, serving as a
    /// pre-send checkpoint so recovery can detect the pending operation.
    async fn persist_outbound_checkpoint(&self, session_id: &str, msg: &Message, mark_sent: bool) {
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
        stream: impl futures::Stream<Item = Result<StreamEvent, E>> + Unpin,
        plugin: &std::sync::Arc<dyn IMPlugin>,
    ) -> Result<StreamResult, GatewayError> {
        self.send_outbound_streaming_inner(session_id, channel, stream, plugin, None)
            .await
    }

    /// Streaming outbound dispatch with session-assembled content blocks.
    ///
    /// When `session_content_blocks` is provided (from
    /// [`SessionStream`](closeclaw_session::llm_session::SessionStream)),
    /// the post-stream pipeline uses them as the source of truth instead
    /// of the Gateway-internal `StreamState` accumulation.
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
    /// `StreamState` accumulation. This ensures the Session layer is the
    /// source of truth for `ContentBlock[]` assembly.
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

    /// Post-stream pipeline: select content blocks, run processor chain,
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

        let processed = self
            .process_or_bypass(
                "",
                content_blocks_for_pipeline,
                channel,
                session_id,
                verbosity_level,
            )
            .await?;

        Ok(StreamResult {
            content_blocks: processed.content_blocks,
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
}

/// Mutable state carried across stream events in `send_outbound_streaming`.
struct StreamState {
    content_blocks: Vec<ContentBlock>,
    usage: UnifiedUsage,
    verbosity_level: VerbosityLevel,
    media_name: Option<String>,
    media_url: Option<String>,
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
            media_name: None,
            media_url: None,
        }
    }

    /// Take the accumulated media block and reset state.
    fn take_media_block(&mut self, block_type: ContentBlockType) -> ContentBlock {
        let name = self.media_name.take().unwrap_or_default();
        let url = self.media_url.take().unwrap_or_default();
        match block_type {
            ContentBlockType::Image => ContentBlock::Image { name, url },
            ContentBlockType::Audio => ContentBlock::Audio { name, url },
            ContentBlockType::File => ContentBlock::File { name, url },
            _ => unreachable!(),
        }
    }
}

/// Send any text messages from `out` into `state`.
///
/// In the incremental streaming phase, text lines are dispatched as-is
/// without DslParser processing. DSL parsing is deferred to the
/// post-stream Processor Chain in `finish_streaming_pipeline`.
async fn dispatch_text(
    ctx: &StreamContext<'_>,
    out: StreamingOutput,
    state: &mut StreamState,
) -> Result<(), GatewayError> {
    for text in out.text_messages {
        tracing::info!(
            chat_id = ctx.chat_id,
            content = %text,
            "streaming outbound text"
        );
        if !text.is_empty() {
            send_text(ctx, &text).await?;
            state.content_blocks.push(ContentBlock::Text(text));
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

/// Filter content blocks based on the session's verbosity level.
///
/// - [`VerbosityLevel::Full`]: no filtering, all blocks are kept.
/// - [`VerbosityLevel::Normal`]: remove [`ContentBlock::Thinking`] blocks.
/// - [`VerbosityLevel::Off`]: only keep [`ContentBlock::Text`] blocks.
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
