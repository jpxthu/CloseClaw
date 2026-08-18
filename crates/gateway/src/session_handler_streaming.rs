//! Streaming LLM call implementation for SessionMessageHandler.
//!
//! Extracted from `session_handler.rs` to keep file sizes under the
//! 500-line project limit.
//!
//! The LLM stream is opened via [`ConversationSession::invoke_llm_streaming`].
//! This file handles Gateway-side orchestration: wrapping the stream with
//! [`SinkUpdater`][closeclaw_llm::SinkUpdater], racing against
//! a cancellation token, and dispatching through
//! [`Gateway::send_outbound_streaming`].

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::session_handler::{MessageMetadata, SessionMessageHandler};
use crate::outbound::StreamResult;
use crate::session_manager::SessionManager;
use crate::types::{GatewayError, Message};
use crate::Gateway;
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::StreamingSink;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::streaming::StreamDone;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::LLMError;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::llm_session::SessionStream;

impl SessionMessageHandler {
    /// Make a streaming LLM call and dispatch it through Gateway's
    /// streaming outbound pipeline.
    ///
    /// Delegates to [`ConversationSession::invoke_llm_streaming`] to
    /// open the raw LLM stream, then handles:
    /// 1. Wrapping the raw LLM stream with
    ///    [`SinkUpdater`][closeclaw_llm::SinkUpdater].
    /// 2. Racing the stream against a cancellation token.
    /// 3. Dispatching through [`Gateway::send_outbound_streaming`].
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn call_llm_streaming(
        cs: &Arc<tokio::sync::RwLock<ConversationSession>>,
        content: &str,
        _meta: &MessageMetadata,
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        channel: &str,
        gateway: &Arc<Gateway>,
        plugin: &Arc<dyn IMPlugin>,
    ) -> Result<StreamResult, LLMError> {
        // ── Open LLM stream via ConversationSession ──
        // Set per-request context for dynamic-layer injection before
        // opening the stream so build_system_prompt_parts sees current metadata.
        cs.read()
            .await
            .set_request_context(_meta.to_request_context());
        let session_stream: SessionStream = cs.write().await.invoke_llm_streaming(content).await?;

        // Retrieve the session's streaming sink (if any) for delta notifications.
        let sink: Option<Arc<dyn closeclaw_llm::streaming::StreamingSink>> =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.streaming_sink().cloned()
            } else {
                None
            };

        // Acquire this session's cancellation token so a streaming
        // request can be aborted mid-stream by a cascade stop.
        let cancel_token: CancellationToken =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.cancel_token().clone()
            } else {
                CancellationToken::new()
            };

        // Wrap the SessionStream with SinkUpdater so the session's
        // StreamingSink (CLI/websocket) still receives per-delta text
        // notifications in parallel with the IM plugin dispatch.
        let wrapped = closeclaw_llm::SinkUpdater::new(session_stream, sink.clone());

        // Race the streaming outbound dispatch against the cancel token.
        let dispatch_result: Result<StreamResult, GatewayError> = tokio::select! {
            res = gateway.send_outbound_streaming(session_id, channel, wrapped, plugin) => res,
            _ = cancel_token.cancelled() => {
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    cs.read().await.set_llm_state(LlmState::Idle);
                }
                if let Some(ref s) = sink {
                    s.send_error("cancelled".to_string());
                }
                tracing::info!(session_id = %session_id, "streaming LLM request cancelled");
                return Err(LLMError::Cancelled);
            }
        };

        // Reset LLM state to Idle after stream completes.
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.read().await.set_llm_state(LlmState::Idle);
        }

        // StreamError degradation path:
        // Send IM notification + persist checkpoint in a single
        // extracted function to keep nesting ≤ 3 levels.
        if let Err(ref e) = dispatch_result {
            if let Err(degrad_err) =
                handle_streaming_degradation(gateway, session_manager, session_id, channel, e).await
            {
                tracing::warn!(
                    session_id = %session_id,
                    error = %degrad_err,
                    "streaming degradation failed"
                );
            }
        }

        let stream_result = dispatch_result.map_err(|e| {
            if let Some(ref s) = sink {
                handle_stream_error(e, s.as_ref())
            } else {
                let msg = e.to_string();
                LLMError::ApiError(msg)
            }
        })?;

        // Best-effort: notify sink of stream completion with usage, so
        // CLI/websocket consumers see a matching `send_done` after the
        // last `send_text` (matching the StreamingSink contract).
        if let Some(ref s) = sink {
            s.send_done(StreamDone {
                model: String::new(),
                usage: Some(stream_result.usage.clone()),
            });
        }

        // If streaming produced no text content blocks, fall back to a
        // single empty text block so the post-LLM completion pipeline
        // (which appends to history) still has something to record.
        if stream_result.content_blocks.is_empty() {
            return Ok(StreamResult {
                content_blocks: vec![ContentBlock::Text(String::new())],
                usage: stream_result.usage,
                retry_attempts: stream_result.retry_attempts,
                dsl_result: stream_result.dsl_result,
            });
        }
        Ok(stream_result)
    }
}

/// Handle streaming error degradation: send IM notification and persist
/// the partial content checkpoint.
///
/// This is an independent async function extracted from
/// `call_llm_streaming` to keep nesting ≤ 3 levels. Uses `let-else`
/// for early returns to reduce nesting.
///
/// # Errors
/// Returns `GatewayError` if chat_id is unavailable or notification
/// fails (logged as warn, not propagated to caller).
pub(crate) async fn handle_streaming_degradation(
    gateway: &Arc<Gateway>,
    session_manager: &Arc<SessionManager>,
    session_id: &str,
    channel: &str,
    dispatch_result: &GatewayError,
) -> Result<(), GatewayError> {
    let GatewayError::StreamError {
        ref partial_content,
        ..
    } = *dispatch_result
    else {
        return Ok(());
    };

    let Some(chat_id) = session_manager.get_chat_id(session_id).await else {
        tracing::warn!(
            session_id = %session_id,
            "chat_id unavailable, skipping streaming degradation"
        );
        return Ok(());
    };

    // IM error notification.
    if let Err(notif_err) = gateway
        .send_outbound_simplified(
            &chat_id,
            channel,
            "⚠️ 回复中断：流式响应异常终止，已发送部分内容可能不完整",
        )
        .await
    {
        tracing::warn!(
            session_id = %session_id,
            error = %notif_err,
            "failed to send streaming error notification \
             via simplified outbound"
        );
    }

    // Checkpoint persistence: write partial content as outbound
    // history with an error marker. Skip if partial_content is
    // empty (nothing was sent before the error).
    if !partial_content.is_empty() {
        persist_streaming_checkpoint(gateway, session_id, &chat_id, channel, partial_content).await;
    }

    Ok(())
}

/// Build a [`Message`] for checkpoint persistence from partial streaming
/// content blocks.
fn build_checkpoint_message(
    chat_id: &str,
    channel: &str,
    partial_content: &[ContentBlock],
) -> Message {
    let text = partial_content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    let content_blocks_json = serde_json::to_string(partial_content).unwrap_or_default();
    Message {
        id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
        from: "agent".to_string(),
        to: chat_id.to_string(),
        content: text,
        channel: channel.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: Some(channel.to_string()),
        dsl_result: None,
        content_blocks: Some(content_blocks_json),
    }
}

/// Persist partial streaming content as an outbound checkpoint.
async fn persist_streaming_checkpoint(
    gateway: &Gateway,
    session_id: &str,
    chat_id: &str,
    channel: &str,
    partial_content: &[ContentBlock],
) {
    let msg = build_checkpoint_message(chat_id, channel, partial_content);
    gateway
        .persist_outbound_checkpoint(session_id, &msg, true)
        .await;
}

/// Handle a streaming error by sending partial content to the user
/// before the error message.
///
/// When a [`GatewayError::StreamError`] occurs mid-stream, any
/// accumulated text blocks are sent to the sink first, then the
/// error message is sent. This ensures the user does not lose
/// already-generated content.
pub(crate) fn handle_stream_error(e: GatewayError, sink: &dyn StreamingSink) -> LLMError {
    let msg = e.to_string();
    if let GatewayError::StreamError {
        ref partial_content,
        ..
    } = e
    {
        for block in partial_content {
            if let ContentBlock::Text(text) = block {
                sink.send_text(text);
            }
        }
        tracing::warn!(
            partial_content_blocks = partial_content.len(),
            "streaming error: partial content blocks preserved"
        );
    }
    sink.send_error(msg.clone());
    LLMError::ApiError(msg)
}
