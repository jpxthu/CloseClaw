//! Streaming outbound helpers and middleware chain functions.
//!
//! Extracted from `outbound.rs` to stay within the 1000-line file limit.

use crate::Gateway;
use crate::GatewayError;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::StreamingOutput;
use closeclaw_common::processor::{ProcessedMessage, ProcessorChain};
use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::UnifiedUsage;
use closeclaw_llm::types::{ContentBlock, ContentBlockType};

/// Bundles the streaming outbound context passed to `process_stream_event` and
/// its sub-handlers. Keeps parameter counts ≤6 (CONTRIBUTING.md limit).
///
/// `session_id` and `channel` are retained as session metadata for potential
/// future use by stream handlers or logging.
///
/// `registry` drives the incremental-phase processor chain: each text chunk
/// and non-text block is processed through VerbosityFilter before dispatch.
#[allow(dead_code)] // read cross-module in outbound.rs
pub(crate) struct StreamContext<'a> {
    pub gateway: &'a Gateway,
    pub plugin: &'a std::sync::Arc<dyn closeclaw_common::im_plugin::IMPlugin>,
    pub session_id: &'a str,
    pub channel: &'a str,
    pub chat_id: &'a str,
    pub thread_id: Option<&'a str>,
    pub registry: Option<&'a std::sync::Arc<dyn closeclaw_common::processor::ProcessorChain>>,
    pub trace_id: Option<&'a str>,
    pub session_key: Option<&'a str>,
}

/// Mutable state carried across stream events in `send_outbound_streaming`.
pub(crate) struct StreamState {
    pub content_blocks: Vec<ContentBlock>,
    pub usage: UnifiedUsage,
    pub verbosity_level: VerbosityLevel,
    pub media_name: Option<String>,
    pub media_url: Option<String>,
    /// DSL instructions accumulated during the incremental phase.
    /// Currently always empty — DslParser is a zero-overhead passthrough in
    /// the incremental phase (design doc). Full DSL parsing is deferred to
    /// the finish phase via `process_outbound_without_verbosity`.
    pub dsl_instructions: Vec<closeclaw_common::processor::DslInstruction>,
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
            dsl_instructions: Vec::new(),
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

/// Log a warning and send a failure notification when batch send fails.
///
/// Called from `dispatch_and_persist` when `plugin.send()` returns an error.
/// Sends a user-facing notification via the simplified path (no retry, no
/// outbound history write) and returns `Ok(())` so the caller can terminate
/// the flow cleanly, matching the design doc §批量出错降级.
pub(crate) async fn notify_batch_send_failure(
    gateway: &Gateway,
    channel: &str,
    chat_id: &str,
    send_error: closeclaw_common::im_plugin::AdapterError,
) {
    tracing::warn!(
        channel,
        chat_id,
        error = %send_error,
        "batch plugin.send failed, sending failure notification"
    );
    let _ = gateway
        .send_outbound_simplified(
            chat_id,
            channel,
            "⚠️ 回复发送失败：消息未能送达，请稍后重试",
        )
        .await;
}

// ---------------------------------------------------------------------------
// Streaming outbound helpers
// ---------------------------------------------------------------------------

/// Send text messages from `outbound` into `state` and dispatch to the user.
///
/// When `ctx.registry` is available, each text chunk is processed through
/// the incremental-phase chain ([`ProcessorChain::process_outbound_incremental`])
/// via [`process_single_through_chain`]. Only VerbosityFilter executes here;
/// DslParser and OutboundRawLog are skipped per design doc.
///
/// When `ctx.registry` is `None`, text passes through unchanged
/// (zero-overhead passthrough).
pub(crate) async fn dispatch_text(
    ctx: &StreamContext<'_>,
    out: StreamingOutput,
    state: &mut StreamState,
) -> Result<(), GatewayError> {
    for text in out.text_messages {
        if let Some(registry) = ctx.registry {
            let block = ContentBlock::Text(text.clone());
            match process_single_through_chain(registry.as_ref(), &block, state.verbosity_level)
                .await
            {
                Ok(processed_blocks) => {
                    for block in &processed_blocks {
                        if let ContentBlock::Text(ref t) = block {
                            if t.is_empty() {
                                continue;
                            }
                            send_text(ctx, t).await?;
                        }
                    }
                    state.content_blocks.extend(processed_blocks);
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "incremental chain failed for text chunk, sending original"
                    );
                    send_text(ctx, &text).await?;
                    state.content_blocks.push(ContentBlock::Text(text));
                }
            }
        } else {
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
    let render_start = std::time::Instant::now();
    let rendered = ctx.plugin.render(std::slice::from_ref(block), None);
    if ctx.channel == "feishu" {
        let render_duration_ms = render_start.elapsed().as_millis() as u64;
        if let Some(trace_id) = ctx.trace_id {
            if !trace_id.is_empty() {
                emit_feishu_render_event(
                    ctx.gateway,
                    trace_id,
                    ctx.session_key,
                    ctx.channel,
                    render_duration_ms,
                );
            }
        }
    }
    tracing::info!(
        chat_id = ctx.chat_id,
        content = ?rendered.payload,
        msg_type = %rendered.msg_type,
        "streaming outbound render block"
    );
    let send_start = std::time::Instant::now();
    ctx.plugin
        .send(&rendered, ctx.chat_id, ctx.thread_id)
        .await?;
    if ctx.channel == "feishu" {
        let send_duration_ms = send_start.elapsed().as_millis() as u64;
        if let Some(trace_id) = ctx.trace_id {
            if !trace_id.is_empty() {
                emit_feishu_send_event(
                    ctx.gateway,
                    trace_id,
                    ctx.session_key,
                    ctx.channel,
                    ctx.chat_id,
                    send_duration_ms,
                );
            }
        }
    }
    Ok(())
}

/// Process a single block through the incremental-phase processor chain.
///
/// Wraps the block in a [`ProcessedMessage`] (with `verbosity_level` metadata)
/// and calls [`ProcessorChain::process_outbound_incremental`]. Returns the
/// processed content blocks on success, or an error on failure (caller
/// handles fallback).
///
/// This is the shared helper used by both [`dispatch_text`] and
/// `process_and_send_non_text_blocks` to eliminate duplicated chain
/// processing logic (E2 review DRY fix).
pub(crate) async fn process_single_through_chain(
    registry: &dyn ProcessorChain,
    block: &ContentBlock,
    verbosity_level: VerbosityLevel,
) -> Result<Vec<ContentBlock>, closeclaw_common::processor::ProcessError> {
    let msg = ProcessedMessage {
        content_blocks: vec![block.clone()],
        metadata: std::collections::HashMap::from([(
            "verbosity_level".to_string(),
            verbosity_level.to_string(),
        )]),
    };
    registry
        .process_outbound_incremental(msg)
        .await
        .map(|processed| processed.content_blocks)
}

/// Build outbound metadata from key-value pairs.
pub(crate) fn make_outbound_meta(
    entries: &[(&str, &str)],
) -> std::collections::HashMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Extract checkpoint content from a rendered output based on msg_type.
///
/// Returns the text for "text" payloads, a JSON string for "interactive",
/// or an error for unknown types. Extracted from [`dispatch_and_persist`]
/// to keep the function body within the 50-line limit.
pub(crate) fn extract_content_for_checkpoint(
    rendered: &RenderedOutput,
    fallback_text: &str,
) -> Result<String, crate::GatewayError> {
    match rendered.msg_type.as_str() {
        "text" => Ok(rendered
            .payload
            .get("content")
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or(fallback_text)
            .to_string()),
        "interactive" => {
            Ok(serde_json::to_string(&rendered.payload).unwrap_or_else(|_| "{}".to_string()))
        }
        _ => Err(crate::GatewayError::OutboundError(format!(
            "unknown msg_type: {}",
            rendered.msg_type
        ))),
    }
}

/// Merge incremental-phase DSL instructions with finish-phase DslParser
/// result from the processor chain metadata.
///
/// When DslParser ran in the finish phase (`metadata` contains
/// `"dsl_result"`), its instructions are appended to the incremental
/// ones. When DslParser did not run but incremental instructions exist,
/// they are serialized directly. Returns `None` when both are empty
/// (no registry or no DSL content).
pub(crate) fn merge_dsl_results(
    metadata: &std::collections::HashMap<String, String>,
    incremental: Vec<closeclaw_common::processor::DslInstruction>,
) -> Option<String> {
    use closeclaw_common::processor::DslParseResult;

    if let Some(raw) = metadata.get("dsl_result") {
        let chain_instructions = serde_json::from_str::<DslParseResult>(raw)
            .ok()
            .map(|r| r.instructions)
            .unwrap_or_default();
        let merged: Vec<_> = incremental.into_iter().chain(chain_instructions).collect();
        let r = DslParseResult {
            instructions: merged,
        };
        serde_json::to_string(&r).ok()
    } else if !incremental.is_empty() {
        let r = DslParseResult {
            instructions: incremental,
        };
        serde_json::to_string(&r).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Feishu debug-log helpers
// ---------------------------------------------------------------------------

/// Emit a `feishu.outbound.rendered` debug event.
pub(crate) fn emit_feishu_render_event(
    gateway: &Gateway,
    trace_id: &str,
    session_key: Option<&str>,
    channel: &str,
    render_duration_ms: u64,
) {
    if trace_id.is_empty() {
        return;
    }
    let guard = gateway.debug_log.read().unwrap_or_else(|e| e.into_inner());
    crate::debug_log_emitter::emit_debug_event(
        guard.as_ref(),
        trace_id,
        session_key,
        closeclaw_debug_log::LogLevel::Info,
        "feishu",
        "feishu.outbound.rendered",
        serde_json::json!({
            "platform": channel,
            "render_duration_ms": render_duration_ms,
        }),
    );
}

/// Emit a `feishu.api.send` debug event.
pub(crate) fn emit_feishu_send_event(
    gateway: &Gateway,
    trace_id: &str,
    session_key: Option<&str>,
    channel: &str,
    peer_id: &str,
    send_duration_ms: u64,
) {
    if trace_id.is_empty() {
        return;
    }
    let guard = gateway.debug_log.read().unwrap_or_else(|e| e.into_inner());
    crate::debug_log_emitter::emit_debug_event(
        guard.as_ref(),
        trace_id,
        session_key,
        closeclaw_debug_log::LogLevel::Info,
        "feishu",
        "feishu.api.send",
        serde_json::json!({
            "platform": channel,
            "peer_id": peer_id,
            "send_duration_ms": send_duration_ms,
        }),
    );
}

// ---------------------------------------------------------------------------
// Convenience outbound methods
// ---------------------------------------------------------------------------

impl Gateway {
    /// Lightweight outbound to a specific chat (no session_id required).
    pub async fn send_outbound_to_chat(
        &self,
        chat_id: &str,
        channel: &str,
        raw_output: &str,
    ) -> Result<(), GatewayError> {
        let Some(plugin) = self.get_plugin(channel).await else {
            return self.fallback_to_plain_text(channel, raw_output).await;
        };
        let blocks = vec![closeclaw_llm::types::ContentBlock::Text(
            raw_output.to_string(),
        )];
        let processed = self
            .process_or_bypass(raw_output, blocks, channel, "", VerbosityLevel::default())
            .await?;
        if processed.content_blocks.is_empty() {
            return Ok(());
        }
        let dsl_result: Option<closeclaw_common::processor::DslParseResult> = processed
            .metadata
            .get("dsl_result")
            .and_then(|s| serde_json::from_str(s).ok());
        let rendered = plugin.render(&processed.content_blocks, dsl_result.as_ref());
        let middlewares = self.get_outbound_middlewares().await;
        if !middlewares.is_empty() {
            let mctx = Gateway::make_middleware_ctx("", channel, chat_id);
            if let Err(e) =
                closeclaw_processor_chain::run_middleware_chain(&middlewares, &mctx, &rendered)
                    .await
            {
                return log_middleware_rejection(self, e, chat_id, channel).await;
            }
        }
        plugin.send(&rendered, chat_id, None).await?;
        Ok(())
    }

    /// Simplified outbound: raw-log → render → send (no Verbosity/DslParser/middleware).
    pub async fn send_outbound_simplified(
        &self,
        chat_id: &str,
        channel: &str,
        raw_output: &str,
    ) -> Result<(), GatewayError> {
        let Some(plugin) = self.get_plugin(channel).await else {
            return self.fallback_to_plain_text(channel, raw_output).await;
        };
        let blocks = vec![closeclaw_llm::types::ContentBlock::Text(
            raw_output.to_string(),
        )];
        let processed = self
            .process_outbound_raw_log_only(raw_output, blocks.clone(), channel)
            .await?;
        if processed.content_blocks.is_empty() {
            return Ok(());
        }
        let rendered = plugin.render(&processed.content_blocks, None);
        if plugin.send(&rendered, chat_id, None).await.is_err() {
            self.send_as_plain_text(&plugin, raw_output, chat_id, None)
                .await
        } else {
            Ok(())
        }
    }
}
