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
use crate::types::GatewayError;
use crate::Gateway;
use closeclaw_common::im_plugin::IMPlugin;
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

        let stream_result = dispatch_result.map_err(|e| {
            let msg = e.to_string();
            if let Some(ref s) = sink {
                s.send_error(msg.clone());
            }
            if let GatewayError::StreamError {
                partial_content, ..
            } = e
            {
                tracing::warn!(
                    partial_content_blocks = partial_content.len(),
                    "streaming error: partial content blocks preserved"
                );
            }
            LLMError::ApiError(msg)
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
            });
        }
        Ok(stream_result)
    }
}
