//! Streaming outbound helpers and middleware chain functions.
//!
//! Extracted from `outbound.rs` to stay within the 1000-line file limit.

use crate::Gateway;
use crate::GatewayError;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::StreamingOutput;
use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::{ContentBlock, ContentBlockType};
use closeclaw_llm::types::UnifiedUsage;

/// Bundles the streaming outbound context passed to `process_stream_event` and
/// its sub-handlers. Keeps parameter counts ≤6 (CONTRIBUTING.md limit).
///
/// `session_id` and `channel` are retained as session metadata for potential
/// future use by stream handlers or logging.
#[allow(dead_code)] // read cross-module in outbound.rs
pub(crate) struct StreamContext<'a> {
    pub plugin: &'a std::sync::Arc<dyn closeclaw_common::im_plugin::IMPlugin>,
    pub session_id: &'a str,
    pub channel: &'a str,
    pub chat_id: &'a str,
    pub thread_id: Option<&'a str>,
}

/// Mutable state carried across stream events in `send_outbound_streaming`.
pub(crate) struct StreamState {
    pub content_blocks: Vec<ContentBlock>,
    pub usage: UnifiedUsage,
    pub verbosity_level: VerbosityLevel,
    pub media_name: Option<String>,
    pub media_url: Option<String>,
}

impl StreamState {
    pub fn new(verbosity_level: VerbosityLevel) -> Self {
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
    pub fn take_media_block(&mut self, block_type: ContentBlockType) -> ContentBlock {
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

/// Log a middleware chain error and send a rejection notification to the
/// user via the simplified path (skips VerbosityFilter/DslParser/middleware).
/// Returns `Ok(())` so the caller can discard the message without
/// propagating the error.
pub(crate) async fn log_middleware_rejection(
    gateway: &Gateway,
    e: closeclaw_common::MiddlewareError,
    chat_id: &str,
    channel: &str,
) -> Result<(), GatewayError> {
    match &e {
        closeclaw_common::MiddlewareError::Rejected { name, reason } => {
            tracing::warn!(
                middleware = %name,
                reason = %reason,
                chat_id,
                "middleware rejected outbound message, discarding"
            );
        }
        closeclaw_common::MiddlewareError::MiddlewareFailed { name, .. } => {
            tracing::warn!(
                middleware = %name,
                chat_id,
                "middleware failed, discarding outbound message"
            );
        }
    }
    // Send a rejection notification to the user via simplified path
    // (skips middleware to avoid re-rejection), consistent with the
    // streaming outbound path.
    let _ = gateway
        .send_outbound_simplified(
            chat_id,
            channel,
            "Your message was not sent due to an outbound policy restriction.",
        )
        .await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Streaming outbound helpers
// ---------------------------------------------------------------------------

/// Send text messages from `out` into `state` (no DslParser processing).
pub(crate) async fn dispatch_text(
    ctx: &StreamContext<'_>,
    out: StreamingOutput,
    state: &mut StreamState,
) -> Result<(), GatewayError> {
    for text in out.text_messages {
        tracing::info!(chat_id = ctx.chat_id, content = %text, "streaming outbound text");
        if !text.is_empty() {
            send_text(ctx, &text).await?;
            state.content_blocks.push(ContentBlock::Text(text));
        }
    }
    Ok(())
}

/// Construct a text [`RenderedOutput`] and dispatch via `plugin.send`.
///
/// Outbound middleware is no longer applied per-chunk during streaming.
/// Instead, a pre-flight check runs once before the stream loop starts
/// (see [`crate::outbound::Gateway::send_outbound_streaming_inner`]).
pub(crate) async fn send_text(ctx: &StreamContext<'_>, text: &str) -> Result<(), GatewayError> {
    let rendered = RenderedOutput {
        msg_type: "text".to_string(),
        payload: serde_json::json!({"content": {"text": text}}),
    };
    ctx.plugin
        .send(&rendered, ctx.chat_id, ctx.thread_id)
        .await
        .map_err(Into::into)
}

/// Render and dispatch via `plugin.send`.
///
/// Outbound middleware is no longer applied per-chunk during streaming.
/// Instead, a pre-flight check runs once before the stream loop starts
/// (see [`crate::outbound::Gateway::send_outbound_streaming_inner`]).
pub(crate) async fn send_render_block(
    ctx: &StreamContext<'_>,
    block: &ContentBlock,
) -> Result<(), GatewayError> {
    let rendered = ctx.plugin.render(std::slice::from_ref(block), None);
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


