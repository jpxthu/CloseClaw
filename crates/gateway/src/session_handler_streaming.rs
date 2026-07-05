//! Streaming LLM call implementation for SessionMessageHandler.
//!
//! Extracted from `session_handler.rs` to keep file sizes under the
//! 500-line project limit.
//!
//! The LLM stream is opened via [`crate::llm_caller_impl::FallbackLlmCaller`].
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
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMError;

impl SessionMessageHandler {
    /// Make a streaming LLM call and dispatch it through Gateway's
    /// streaming outbound pipeline.
    ///
    /// Opens the LLM stream via [`FallbackLlmCaller`], then handles:
    /// 1. Wrapping the raw LLM stream with
    ///    [`SinkUpdater`][closeclaw_llm::SinkUpdater].
    /// 2. Racing the stream against a cancellation token.
    /// 3. Dispatching through [`Gateway::send_outbound_streaming`].
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn call_llm_streaming(
        unified_fallback_client: &Arc<UnifiedFallbackClient>,
        content: &str,
        _meta: &MessageMetadata,
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        channel: &str,
        gateway: &Arc<Gateway>,
        plugin: &Arc<dyn IMPlugin>,
    ) -> Result<StreamResult, LLMError> {
        use closeclaw_common::LlmCaller;
        use closeclaw_llm::session::InjectionPosition;
        use closeclaw_llm::types::InternalMessage;

        // ── Build request with memory injection ──
        let mut messages = vec![InternalMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_call_id: None,
        }];
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let inj = { cs.read().await.take_memory_injection() };
            if let Some(injection) = inj {
                let tool_msg = InternalMessage {
                    role: "tool".to_string(),
                    content: injection.content.clone(),
                    tool_call_id: None,
                };
                match injection.position_mode {
                    InjectionPosition::AfterCurrent => {
                        messages.push(tool_msg);
                    }
                    InjectionPosition::BeforeNext => {
                        messages.insert(0, tool_msg);
                    }
                }
            }
        }
        let request = closeclaw_llm::types::InternalRequest {
            model: String::new(),
            messages,
            temperature: 0.7,
            max_tokens: None,
            stream: true,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: closeclaw_session::persistence::ReasoningLevel::default(),
            turn_count: None,
        };

        // ── Open LLM stream via FallbackLlmCaller ──
        let caller = crate::llm_caller_impl::FallbackLlmCaller(Arc::clone(unified_fallback_client));
        let raw_stream = caller.call_streaming(request).await?;

        // Retrieve the session's streaming sink (if any) for delta notifications.
        let sink: Option<Arc<dyn closeclaw_llm::streaming::StreamingSink>> =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.streaming_sink().map(|s| s.clone())
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

        // Wrap the raw LLM stream with SinkUpdater so the session's
        // StreamingSink (CLI/websocket) still receives per-delta text
        // notifications in parallel with the IM plugin dispatch in
        // `send_outbound_streaming`.
        let wrapped = closeclaw_llm::SinkUpdater::new(raw_stream, sink.clone());

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
