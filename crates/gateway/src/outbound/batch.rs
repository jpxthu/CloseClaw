//! Batch-mode outbound dispatch: full processor chain → render →
//! middleware → send → checkpoint persistence, plus plain-text fallbacks.

use crate::outbound_helpers::{
    log_middleware_rejection, notify_batch_send_failure, CheckpointMeta,
};
use crate::outbound_middleware::runner::run_middleware_chain;
use crate::{Gateway, GatewayError, Message};
use closeclaw_common::im_plugin::{IMPlugin, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::MiddlewareContext;
use closeclaw_llm::types::ContentBlock;
use std::sync::Arc;

use super::types::{SendOutboundIds, SendOutcome};

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
    /// `finish_streaming_pipeline`).
    pub async fn send_outbound(
        &self,
        session_id: &str,
        channel: &str,
        raw_output: &str,
        content_blocks: Vec<ContentBlock>,
        ids: SendOutboundIds,
    ) -> Result<SendOutcome, GatewayError> {
        let SendOutboundIds {
            trace_id,
            session_key,
        } = ids;
        // 1. Resolve chat_id and plugin.
        let chat_id = self.resolve_outbound_chat_id(session_id).await?;
        let Some(plugin) = self.get_plugin(channel).await else {
            return self
                .fallback_to_plain_text(channel, raw_output)
                .await
                .map(|()| SendOutcome::Sent);
        };

        // 2. Resolve verbosity level and inject into chain metadata.
        let verbosity_level = self.resolve_outbound_verbosity_level(session_id).await;

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
        let routing = self.resolve_outbound_thread_routing(session_id).await;
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
            thread_id: routing.thread_id,
            reply_ref: routing.reply_ref,
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
        let middlewares = self.get_outbound_middlewares().await;
        if !middlewares.is_empty() {
            let mctx = Self::make_middleware_ctx(ctx.session_id, ctx.channel, &ctx.chat_id);
            if let Err(e) = run_middleware_chain(&middlewares, &mctx, ctx.rendered).await {
                return log_middleware_rejection(e, &ctx.chat_id)
                    .await
                    .map(|()| SendOutcome::Sent);
            }
        }
        let message_id = format!("out-{}", chrono::Utc::now().timestamp_millis());
        if let Err(e) = self
            .run_write_ahead(ctx.session_id, &message_id, ctx.channel)
            .await
        {
            tracing::warn!(
                session_id = ctx.session_id,
                message_id = %message_id,
                error = %e,
                "write-ahead persist failed, skipping plugin.send"
            );
            return Ok(SendOutcome::Notified);
        }
        self.send_and_record(ctx, &message_id).await
    }

    /// Write-ahead: record OutboundMessage pending op before send.
    ///
    /// Per design doc: persistence must succeed before the actual send.
    /// On failure the caller must NOT proceed with `plugin.send`.
    async fn run_write_ahead(
        &self,
        session_id: &str,
        message_id: &str,
        channel: &str,
    ) -> Result<(), GatewayError> {
        crate::outbound_helpers::record_outbound_pending_op(self, session_id, message_id, channel)
            .await
    }

    /// Send via plugin, then ack-clear and persist the outbound checkpoint.
    async fn send_and_record(
        &self,
        ctx: DispatchCtx<'_>,
        message_id: &str,
    ) -> Result<SendOutcome, GatewayError> {
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
                crate::outbound_helpers::SendDebugCtx {
                    gateway: self,
                    trace_id: ctx.trace_id.as_deref(),
                    session_key: ctx.session_key.as_deref(),
                    parent: None,
                },
                ctx.channel,
                &ctx.chat_id,
                send_duration_ms,
            );
        }
        if let Err(e) = send_result {
            notify_batch_send_failure(self, ctx.channel, &ctx.chat_id, e).await;
            return Ok(SendOutcome::Notified);
        }
        self.ack_clear_and_record(ctx, message_id).await
    }

    /// Ack-clear: remove OutboundMessage pending op after successful send,
    /// then persist the outbound delivery record to checkpoint.
    async fn ack_clear_and_record(
        &self,
        ctx: DispatchCtx<'_>,
        message_id: &str,
    ) -> Result<SendOutcome, GatewayError> {
        crate::outbound_helpers::clear_outbound_pending_op(self, ctx.session_id, message_id).await;
        let content = crate::outbound_helpers::extract_content_for_checkpoint(
            ctx.rendered,
            ctx.fallback_text,
        )?;
        let msg = Self::make_outbound_msg(
            ctx.channel,
            ctx.chat_id.clone(),
            message_id.to_string(),
            content,
            CheckpointMeta {
                platform: Some(ctx.channel.to_string()),
                dsl_result: ctx.dsl_result.clone(),
                content_blocks: ctx.content_blocks.clone(),
            },
        );
        crate::outbound_helpers::persist_outbound_checkpoint(self, ctx.session_id, &msg, true)
            .await;
        crate::outbound_helpers::emit_send_completed_log(
            crate::outbound_helpers::SendDebugCtx {
                gateway: self,
                trace_id: ctx.trace_id.as_deref(),
                session_key: ctx.session_key.as_deref(),
                parent: None,
            },
            ctx.channel,
            &ctx.chat_id,
        );
        Ok(SendOutcome::Sent)
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
    #[allow(dead_code)] // kept for test assertions; no longer called in production path
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
    ///
    /// `pub(crate)` so the aggregate-struct unit tests can assert the
    /// `CheckpointMeta` → `Message` field mapping directly (same crate-level
    /// visibility as the other outbound helpers).
    pub(crate) fn make_outbound_msg(
        channel: &str,
        to: String,
        id: String,
        content: String,
        meta: CheckpointMeta,
    ) -> Message {
        Message {
            id,
            from: "agent".to_string(),
            to,
            content,
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
            thread_id: None,
            reply_ref: None,
            platform: meta.platform,
            dsl_result: meta.dsl_result,
            content_blocks: meta.content_blocks,
        }
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
