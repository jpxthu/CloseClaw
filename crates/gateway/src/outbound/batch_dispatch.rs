//! Batch-mode dispatch pipeline: outbound middleware gate, write-ahead
//! pending op, plugin send, and outbound checkpoint persistence for a
//! rendered output (the tail of the batch flow after rendering).

use crate::outbound_helpers::{
    log_middleware_rejection, notify_batch_send_failure, CheckpointMeta,
};
use crate::outbound_middleware::runner::run_middleware_chain;
use crate::{Gateway, GatewayError, Message};
use closeclaw_common::im_plugin::{IMPlugin, RenderedOutput};
use closeclaw_common::MiddlewareContext;

use super::types::SendOutcome;

/// Per-call context for dispatching a rendered output and persisting its
/// checkpoint. Bundled into a struct to keep the helper's parameter list short.
pub(super) struct DispatchCtx<'a> {
    pub(super) plugin: &'a std::sync::Arc<dyn IMPlugin>,
    pub(super) rendered: &'a RenderedOutput,
    /// Plain-text fallback used when the rendered payload does not carry a
    /// `content.text` field. Typically the processed chain's `content`.
    pub(super) fallback_text: &'a str,
    pub(super) session_id: &'a str,
    pub(super) channel: &'a str,
    pub(super) chat_id: String,
    /// Optional thread/topic ID for directing the message into a thread.
    pub(super) thread_id: Option<String>,
    /// Outbound directed reference for定向回复.
    pub(super) reply_ref: Option<String>,
    /// DSL result string from the processor chain (JSON serialized).
    pub(super) dsl_result: Option<String>,
    /// Serialized content blocks (JSON) for checkpoint persistence.
    pub(super) content_blocks: Option<String>,
    /// Inbound trace_id for debug log event correlation.
    pub(super) trace_id: Option<String>,
    /// Inbound session_key for debug log event correlation.
    pub(super) session_key: Option<String>,
}

impl Gateway {
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
    pub(super) async fn dispatch_and_persist(
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
}

impl Gateway {
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
}

impl Gateway {
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
