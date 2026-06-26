//! Streaming LLM call implementation for SessionMessageHandler.
//!
//! Extracted from `session_handler.rs` to keep file sizes under the
//! 500-line project limit.
//!
//! The actual System Prompt construction and LLM stream opening are
//! delegated to [`crate::session::llm_caller`]. This file only handles
//! the Gateway-side orchestration: wrapping the stream with
//! [`SinkUpdater`][crate::session::llm_caller::SinkUpdater], racing
//! against a cancellation token, and dispatching through
//! [`Gateway::send_outbound_streaming`].

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::session_handler::{MessageMetadata, SessionMessageHandler};
use crate::gateway::outbound::StreamResult;
use crate::gateway::session_manager::SessionManager;
use crate::gateway::Gateway;
use crate::im::IMPlugin;
use crate::llm::client::UnifiedChatClient;
use crate::llm::session_state::LlmState;
use crate::llm::streaming::StreamDone;
use crate::llm::types::ContentBlock;
use crate::llm::LLMError;

impl SessionMessageHandler {
    /// Make a streaming LLM call and dispatch it through Gateway's
    /// streaming outbound pipeline.
    ///
    /// Delegates prompt construction and LLM stream opening to
    /// [`crate::session::llm_caller`]. This method only handles:
    /// 1. Wrapping the raw LLM stream with
    ///    [`SinkUpdater`][crate::session::llm_caller::SinkUpdater].
    /// 2. Racing the stream against a cancellation token.
    /// 3. Dispatching through [`Gateway::send_outbound_streaming`].
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn call_llm_streaming(
        unified_client: &Arc<UnifiedChatClient>,
        content: &str,
        meta: &MessageMetadata,
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        channel: &str,
        gateway: &Arc<Gateway>,
        plugin: &Arc<dyn IMPlugin>,
    ) -> Result<StreamResult, LLMError> {
        // ── Delegate prompt building + LLM stream opening to Session layer ──
        let (raw_stream, sink) = crate::session::llm_caller::call_llm_streaming(
            unified_client,
            content,
            meta,
            session_manager,
            session_id,
        )
        .await?;

        // Acquire this session's cancellation token so a streaming
        // request can be aborted mid-stream by a cascade stop.
        let cancel_token: CancellationToken =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.cancel_token().clone()
            } else {
                CancellationToken::new()
            };

        // Wrap the raw LLM stream with SinkUpdater so the session's
        // StreamingSink (CLI/websocket) still receives per-delta text
        // notifications in parallel with the IM plugin dispatch in
        // `send_outbound_streaming`.
        let wrapped = crate::session::llm_caller::SinkUpdater::new(
            raw_stream,
            sink.clone(),
            Arc::clone(session_manager),
            session_id.to_string(),
        );

        // Race the streaming outbound dispatch against the cancel token.
        let dispatch_result = tokio::select! {
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
            });
        }
        Ok(stream_result)
    }
}
