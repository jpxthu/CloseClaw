//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`](crate::im::IMPlugin) registry.

use super::{Gateway, GatewayError, Message};
use crate::im::IMPlugin;
use crate::llm::types::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedResponse, UnifiedUsage,
};
use crate::processor_chain::dsl_parser::DslParser;
use crate::processor_chain::{DslParseResult, ProcessedMessage};
use crate::renderer::streaming::{DefaultStreamingRenderer, StreamingOutput, StreamingRenderer};
use crate::renderer::RenderedOutput;
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
    /// Flow (single path; legacy renderer / processor-only / bypass branches
    /// are unified here):
    /// 1. Resolve `chat_id` from `session_id` via `SessionManager::get_chat_id`.
    /// 2. Resolve the [`IMPlugin`](super::im::IMPlugin) registered for `channel`
    ///    through `self.plugins`.
    /// 3. Run the processor chain (if `processor_registry` is configured) to
    ///    produce a [`ProcessedMessage`]; otherwise bypass with a synthetic
    ///    `ProcessedMessage` wrapping the raw input.
    /// 4. Honor `processed.suppress` — return `Ok(())` without sending.
    /// 5. Extract `dsl_result` from `processed.metadata["dsl_result"]` (stored
    ///    as a JSON-encoded string by the DSL processor).
    /// 6. Call `plugin.render(blocks, dsl_result)` to obtain a
    ///    [`RenderedOutput`](crate::renderer::RenderedOutput); fall back to a
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

        // 2. Run processor chain (or bypass) and honor suppress.
        let processed = self.process_or_bypass(raw_output, content_blocks).await?;
        if processed.suppress {
            return Ok(());
        }

        // 3. Extract dsl_result (serialized as a JSON string by DslParser).
        let dsl_result: Option<DslParseResult> = processed
            .metadata
            .get("dsl_result")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str(s).ok());

        // 4. Render via the plugin; fall back to a single Text block when
        // content_blocks is empty.
        let rendered = {
            let owned_fallback;
            let blocks: &[ContentBlock] = if processed.content_blocks.is_empty() {
                owned_fallback = vec![ContentBlock::Text(processed.content.clone())];
                &owned_fallback
            } else {
                &processed.content_blocks
            };
            plugin.render(blocks, dsl_result.as_ref())
        };

        // 5. Resolve thread_id from session checkpoint.
        let thread_id = self.session_manager.get_thread_id(session_id).await;

        // 6. Dispatch by msg_type and persist checkpoint on success.
        self.dispatch_and_persist(DispatchCtx {
            plugin: &plugin,
            rendered: &rendered,
            fallback_text: &processed.content,
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
    async fn dispatch_and_persist(&self, ctx: DispatchCtx<'_>) -> Result<(), GatewayError> {
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
                let msg = Self::make_outbound_msg(ctx.channel, ctx.chat_id.clone(), text);
                ctx.plugin
                    .send(ctx.rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                self.persist_outbound_checkpoint(ctx.session_id, &msg).await;
                Ok(())
            }
            "interactive" => {
                let payload_str = serde_json::to_string(&ctx.rendered.payload)
                    .unwrap_or_else(|_| "{}".to_string());
                ctx.plugin
                    .send(ctx.rendered, &ctx.chat_id, ctx.thread_id.as_deref())
                    .await?;
                let msg = Self::make_outbound_msg(ctx.channel, ctx.chat_id, payload_str);
                self.persist_outbound_checkpoint(ctx.session_id, &msg).await;
                Ok(())
            }
            _ => Err(GatewayError::OutboundError(format!(
                "unknown msg_type: {}",
                ctx.rendered.msg_type
            ))),
        }
    }

    /// Run the outbound processor chain if configured, otherwise bypass with
    /// a synthetic [`ProcessedMessage`] wrapping the raw input.
    async fn process_or_bypass(
        &self,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
    ) -> Result<ProcessedMessage, GatewayError> {
        let Some(ref registry) = self.processor_registry else {
            return Ok(ProcessedMessage {
                content: raw_output.to_string(),
                metadata: serde_json::Map::new(),
                suppress: false,
                content_blocks,
            });
        };
        let input = ProcessedMessage {
            content: raw_output.to_string(),
            metadata: serde_json::Map::new(),
            suppress: false,
            content_blocks,
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
            Ok(None) => crate::session::persistence::SessionCheckpoint::new(session_id.to_string()),
            Err(e) => {
                tracing::warn!(session_id, "failed to load checkpoint: {}", e);
                return;
            }
        };
        let mut pending =
            crate::session::persistence::PendingMessage::new(msg.id.clone(), msg.content.clone());
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
            cp.system_appends = cs.system_appends().to_vec();
        }
        cp.touch();
        if let Err(e) = cm.save(cp).await {
            tracing::warn!(session_id, "failed to save checkpoint: {}", e);
        }
    }

    /// Send a streaming LLM response via the registered IM plugin.
    ///
    /// Drives a [`DefaultStreamingRenderer`] over the [`StreamEvent`] stream,
    /// dispatching incremental output to `plugin` as it becomes available:
    /// - Text delta → line buffer → complete lines → `plugin.send` (text)
    /// - BlockEnd (non-Text) → `plugin.render(&[block], None)` → `plugin.send`
    /// - MessageEnd → flush + `DslParser::parse` on accumulated DSL lines →
    ///   `plugin.render` + `plugin.send`
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

        let mut state = StreamState::new();
        while let Some(event_result) = stream.next().await {
            let event = event_result.map_err(|e| GatewayError::OutboundError(e.to_string()))?;
            self.process_stream_event(plugin, &chat_id, thread_id.as_deref(), event, &mut state)
                .await?;
        }
        tracing::debug!(session_id, channel, "streaming outbound complete");
        Ok(StreamState::into_result(state))
    }

    /// Process a single [`StreamEvent`] and update `state`.
    ///
    /// Split from `send_outbound_streaming` to keep the main loop under the
    /// 50-line helper cap.
    async fn process_stream_event(
        &self,
        plugin: &std::sync::Arc<dyn IMPlugin>,
        chat_id: &str,
        thread_id: Option<&str>,
        event: StreamEvent,
        state: &mut StreamState,
    ) -> Result<(), GatewayError> {
        match event {
            StreamEvent::BlockDelta { delta, .. } => {
                let is_text_delta = matches!(delta, ContentDelta::Text { .. });
                // Reconstruct event for the renderer (index is unused for BlockDelta).
                let out = state
                    .renderer
                    .handle_event(StreamEvent::BlockDelta { index: 0, delta });
                // For Text deltas, the renderer may emit completed text lines
                // and dsl lines; non-Text deltas only update internal state.
                if is_text_delta {
                    dispatch_text_and_dsl(plugin, chat_id, thread_id, out, state).await?;
                }
            }
            StreamEvent::BlockEnd { block_type, .. } => {
                let mut out = state.renderer.handle_event(event);
                if block_type != ContentBlockType::Text {
                    let render_blocks = std::mem::take(&mut out.render_blocks);
                    for block in render_blocks {
                        send_render_block(plugin, chat_id, thread_id, &block).await?;
                        state.content_blocks.push(block);
                    }
                }
                dispatch_text_and_dsl(plugin, chat_id, thread_id, out, state).await?;
            }
            StreamEvent::MessageEnd { usage, .. } => {
                let mut out = state.renderer.flush();
                let render_blocks = std::mem::take(&mut out.render_blocks);
                dispatch_text_and_dsl(plugin, chat_id, thread_id, out, state).await?;
                for block in render_blocks {
                    send_render_block(plugin, chat_id, thread_id, &block).await?;
                    state.content_blocks.push(block);
                }
                send_dsl_lines(plugin, chat_id, thread_id, &state.dsl_lines).await?;
                if let Some(u) = usage {
                    state.usage = u;
                }
            }
            StreamEvent::Error { message } => {
                return Err(GatewayError::OutboundError(message));
            }
            StreamEvent::BlockStart { .. } => {
                state.renderer.handle_event(event);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Streaming outbound helpers
// ---------------------------------------------------------------------------

/// Mutable state carried across stream events in `send_outbound_streaming`.
struct StreamState {
    renderer: DefaultStreamingRenderer,
    content_blocks: Vec<ContentBlock>,
    dsl_lines: Vec<String>,
    usage: UnifiedUsage,
}

impl StreamState {
    fn new() -> Self {
        Self {
            renderer: DefaultStreamingRenderer::new(),
            content_blocks: Vec::new(),
            dsl_lines: Vec::new(),
            usage: UnifiedUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
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

/// Send any text messages from `out` and accumulate dsl lines into `state`.
async fn dispatch_text_and_dsl(
    plugin: &std::sync::Arc<dyn IMPlugin>,
    chat_id: &str,
    thread_id: Option<&str>,
    out: StreamingOutput,
    state: &mut StreamState,
) -> Result<(), GatewayError> {
    for text in out.text_messages {
        send_text(plugin, chat_id, thread_id, &text).await?;
        state.content_blocks.push(ContentBlock::Text(text));
    }
    state.dsl_lines.extend(out.dsl_lines);
    Ok(())
}

/// Construct a text [`RenderedOutput`] and dispatch via `plugin.send`.
async fn send_text(
    plugin: &std::sync::Arc<dyn IMPlugin>,
    chat_id: &str,
    thread_id: Option<&str>,
    text: &str,
) -> Result<(), GatewayError> {
    let rendered = RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!({ "content": { "text": text } }),
    };
    plugin.send(&rendered, chat_id, thread_id).await?;
    Ok(())
}

/// Call `plugin.render(&[block], None)` and dispatch via `plugin.send`.
async fn send_render_block(
    plugin: &std::sync::Arc<dyn IMPlugin>,
    chat_id: &str,
    thread_id: Option<&str>,
    block: &ContentBlock,
) -> Result<(), GatewayError> {
    let rendered = plugin.render(std::slice::from_ref(block), None);
    plugin.send(&rendered, chat_id, thread_id).await?;
    Ok(())
}

/// Parse accumulated DSL lines and dispatch via `plugin.render + plugin.send`.
async fn send_dsl_lines(
    plugin: &std::sync::Arc<dyn IMPlugin>,
    chat_id: &str,
    thread_id: Option<&str>,
    dsl_lines: &[String],
) -> Result<(), GatewayError> {
    if dsl_lines.is_empty() {
        return Ok(());
    }
    let dsl_text = dsl_lines.join("\n");
    let dsl_result = DslParser.parse(&dsl_text);
    let blocks = vec![ContentBlock::Text(dsl_result.clean_content.clone())];
    let rendered = plugin.render(&blocks, Some(&dsl_result));
    plugin.send(&rendered, chat_id, thread_id).await?;
    Ok(())
}
