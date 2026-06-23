//! Streaming LLM call implementation for SessionMessageHandler.
//!
//! Extracted from `session_handler.rs` to keep file sizes under the
//! 500-line project limit.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use tokio_util::sync::CancellationToken;

use super::session_handler::{MessageMetadata, SessionMessageHandler};
use crate::gateway::outbound::StreamResult;
use crate::gateway::session_manager::SessionManager;
use crate::gateway::Gateway;
use crate::im::IMPlugin;
use crate::llm::client::UnifiedChatClient;
use crate::llm::protocol::ProtocolError;
use crate::llm::session::ChatSession;
use crate::llm::session_state::LlmState;
use crate::llm::streaming::{StreamDone, StreamingSink};
use crate::llm::types::{ContentBlock, ContentDelta, StreamEvent};
use crate::llm::{LLMError, Message as ChatMessage};
use crate::session::persistence::ReasoningLevel;
use crate::system_prompt::inject::{
    build_dynamic_sections, build_full_system_prompt, split_static_dynamic,
};

impl SessionMessageHandler {
    /// Make a streaming LLM call and dispatch it through Gateway's
    /// streaming outbound pipeline.
    ///
    /// The function:
    /// 1. Builds the request (system prompt + user message).
    /// 2. Opens the LLM stream and wraps it with [`SinkUpdater`] so the
    ///    session's [`StreamingSink`] (CLI/websocket consumers) still
    ///    receives per-delta text notifications.
    /// 3. Calls [`Gateway::send_outbound_streaming`] which drives the
    ///    [`crate::renderer::streaming::DefaultStreamingRenderer`] and
    ///    dispatches incremental output to `plugin` via the IM platform.
    /// 4. Returns the accumulated [`StreamResult`] (content blocks + usage)
    ///    for the post-LLM completion pipeline.
    ///
    /// Cancellation: the LLM call is raced against `cancel_token.cancelled()`
    /// so a cascade stop can abort the in-flight stream.
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
        let (
            static_prompt_opt,
            session_timestamp,
            turn_count,
            workdir_path,
            system_appends,
            reasoning_level,
        ) = if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let cs_read = cs.read().await;
            (
                cs_read.system_prompt().map(|s| s.to_string()),
                Some(cs_read.session_created_at()),
                cs_read.turn_count(),
                cs_read.workdir().to_string_lossy().into_owned(),
                cs_read.system_appends().to_vec(),
                cs_read.reasoning_level(),
            )
        } else {
            (
                None,
                None,
                0,
                String::new(),
                Vec::new(),
                ReasoningLevel::default(),
            )
        };

        let dynamic_sections = build_dynamic_sections(
            meta,
            Some(workdir_path.as_str()),
            &system_appends,
            session_timestamp,
        );
        let overrides = session_manager.get_prompt_overrides().await;
        let full_prompt = build_full_system_prompt(
            static_prompt_opt.as_deref(),
            &dynamic_sections,
            overrides.as_ref(),
        );

        let (system_static, system_dynamic) = split_static_dynamic(&full_prompt);

        let mut messages = vec![];
        if !full_prompt.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: full_prompt,
            });
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });

        let internal_request = crate::llm::types::InternalRequest {
            model: String::new(),
            messages: messages
                .iter()
                .map(|m| crate::llm::types::InternalMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                })
                .collect(),
            temperature: 0.7,
            max_tokens: None,
            stream: true,
            extra_body: Default::default(),
            system_static,
            system_dynamic,
            system_blocks: None,
            session_id: Some(session_id.to_string()),
            reasoning_level,
            turn_count: Some(turn_count),
        };

        // Get streaming sink from session (CLI/websocket consumers).
        let sink: Option<Arc<dyn StreamingSink>> =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs_read = cs.read().await;
                cs_read.streaming_sink().cloned()
            } else {
                None
            };

        // Set LLM state to Requesting before dispatching the stream.
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.read().await.set_llm_state(LlmState::Requesting);
        }

        let stream = match unified_client.chat_streaming(internal_request).await {
            Ok(s) => s,
            Err(e) => {
                let msg = e.to_string();
                tracing::error!(error = %msg, "streaming LLM call failed");
                if let Some(ref s) = sink {
                    s.send_error(msg.clone());
                }
                // Reset LLM state on error.
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    cs.read().await.set_llm_state(LlmState::Idle);
                }
                return Err(LLMError::ApiError(msg));
            }
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
        let wrapped = SinkUpdater::new(
            stream,
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

// ---------------------------------------------------------------------------
// SinkUpdater: stream wrapper that mirrors LLM events to the session's
// StreamingSink while forwarding them downstream to the renderer.
// ---------------------------------------------------------------------------

/// Wraps an LLM [`Stream`] so each event is mirrored to the session's
/// [`StreamingSink`] before being forwarded to the downstream consumer
/// (the IM plugin in `send_outbound_streaming`).
///
/// - `BlockDelta(Text)` → `sink.send_text` (and triggers the
///   `Requesting → Receiving` state transition on the first text delta).
/// - `MessageEnd` → `sink.send_done` with the final usage.
/// - `Error` → `sink.send_error`.
/// - Protocol/transport errors → `sink.send_error` with the stringified
///   error message.
///
/// `StreamEvent` derives `Clone` so the wrapper can emit a clone of each
/// event after performing side effects on the original.
struct SinkUpdater<S> {
    inner: S,
    sink: Option<Arc<dyn StreamingSink>>,
    session_manager: Arc<SessionManager>,
    session_id: String,
    first_token: bool,
}

impl<S> SinkUpdater<S> {
    fn new(
        inner: S,
        sink: Option<Arc<dyn StreamingSink>>,
        session_manager: Arc<SessionManager>,
        session_id: String,
    ) -> Self {
        Self {
            inner,
            sink,
            session_manager,
            session_id,
            first_token: true,
        }
    }
}

impl<S> Stream for SinkUpdater<S>
where
    S: Stream<Item = Result<StreamEvent, ProtocolError>> + Unpin,
{
    type Item = Result<StreamEvent, ProtocolError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                match &event {
                    StreamEvent::BlockDelta {
                        delta: ContentDelta::Text { text },
                        ..
                    } => {
                        if self.first_token {
                            self.first_token = false;
                            let sm = Arc::clone(&self.session_manager);
                            let sid = self.session_id.clone();
                            tokio::spawn(async move {
                                if let Some(cs) = sm.get_conversation_session(&sid).await {
                                    cs.read().await.set_llm_state(LlmState::Receiving);
                                }
                            });
                        }
                        if let Some(ref sink) = self.sink {
                            sink.send_text(text);
                        }
                    }
                    StreamEvent::MessageEnd { usage, .. } => {
                        if let Some(ref sink) = self.sink {
                            sink.send_done(StreamDone {
                                model: String::new(),
                                usage: usage.clone(),
                            });
                        }
                    }
                    StreamEvent::Error { message } => {
                        if let Some(ref sink) = self.sink {
                            sink.send_error(message.clone());
                        }
                    }
                    _ => {}
                }
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(Some(Err(e))) => {
                let msg = e.to_string();
                if let Some(ref sink) = self.sink {
                    sink.send_error(msg);
                }
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
