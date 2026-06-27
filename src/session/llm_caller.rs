//! LLM caller — builds requests and calls the LLM from the Session layer.
//!
//! Extracted from `gateway::session_handler` and
//! `gateway::session_handler_streaming` to enforce the design doc boundary:
//! Gateway does **not** build System Prompts or call LLM Providers directly.
//! All LLM interaction is delegated to this Session-layer module.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::system_prompt::inject::{
    build_dynamic_sections, build_full_system_prompt, split_static_dynamic,
};
use closeclaw_gateway::session_handler::MessageMetadata;
use closeclaw_llm::client::UnifiedChatClient;
use closeclaw_llm::session::{ChatSession, ConversationSession};
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::streaming::StreamingSink;
use closeclaw_llm::types::{InternalRequest, UnifiedResponse};
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::{LLMError, Message as ChatMessage};
use closeclaw_session::persistence::ReasoningLevel;

use closeclaw_gateway::session_manager::SessionManager;

// ── Memory injection ───────────────────────────────────────────────

/// Consume the `memory_injection` slot and push user + optional tool
/// messages into `messages` according to the injection position mode.
///
/// When an injection is present:
/// - `AfterCurrent` → `[user(content), tool(injection)]`
/// - `BeforeNext`   → `[tool(injection), user(content)]`
///
/// When absent → `[user(content)]`.
pub async fn push_messages_with_injection(
    messages: &mut Vec<ChatMessage>,
    session_manager: &SessionManager,
    session_id: &str,
    content: &str,
) {
    use closeclaw_llm::session::InjectionPosition;

    let injection: Option<closeclaw_llm::session::MemoryInjection> =
        match session_manager.get_conversation_session(session_id).await {
            Some(cs) => cs.read().await.take_memory_injection(),
            None => None,
        };

    if let Some(inj) = injection {
        match inj.position_mode {
            InjectionPosition::AfterCurrent => {
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: content.to_string(),
                });
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: inj.content,
                });
            }
            InjectionPosition::BeforeNext => {
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: inj.content,
                });
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: content.to_string(),
                });
            }
        }
    } else {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });
    }
}

// ── Request building ────────────────────────────────────────────────

/// Build an [`InternalRequest`] for a chat turn.
///
/// Constructs the System Prompt, merges user content (with optional
/// memory injection), and returns the fully populated request.
///
/// When `stream = true` the [`UnifiedChatClient::chat_streaming`] entry
/// point should be used; otherwise [`UnifiedFallbackClient::chat`].
pub async fn build_request(
    session_manager: &SessionManager,
    session_id: &str,
    meta: &MessageMetadata,
    content: &str,
    stream: bool,
) -> Result<InternalRequest, LLMError> {
    let (
        static_prompt_opt,
        session_timestamp,
        turn_count,
        workdir_path,
        system_appends,
        reasoning_level,
    ) = if let Some(cs) = session_manager.get_conversation_session(session_id).await {
        let cs_read: tokio::sync::RwLockReadGuard<'_, ConversationSession> = cs.read().await;
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

    // ── Dynamic sections ───────────────────────────────────────────
    let dynamic_sections = build_dynamic_sections(
        meta,
        Some(workdir_path.as_str()),
        &system_appends,
        session_timestamp,
    );

    // ── Compose full prompt ─────────────────────────────────────────
    let overrides = session_manager.get_prompt_overrides().await;
    let full_prompt = build_full_system_prompt(
        static_prompt_opt.as_deref(),
        &dynamic_sections,
        overrides.as_ref(),
    );

    // ── Split static/dynamic for cache adapter ─────────────────────
    let (system_static, system_dynamic) = split_static_dynamic(&full_prompt);

    // ── Build messages with system + user ───────────────────────────
    let mut messages = vec![];
    if !full_prompt.is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: full_prompt,
        });
    }

    // ── Consume memory_injection slot ──────────────────────────────
    push_messages_with_injection(&mut messages, session_manager, session_id, content).await;

    Ok(InternalRequest {
        model: String::new(),
        messages: messages
            .iter()
            .map(|m| closeclaw_llm::types::InternalMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect(),
        temperature: 0.7,
        max_tokens: None,
        stream,
        extra_body: Default::default(),
        system_static,
        system_dynamic,
        system_blocks: None,
        tools: None,
        session_id: Some(session_id.to_string()),
        reasoning_level,
        turn_count: Some(turn_count),
    })
}

// ── Non-streaming LLM call ─────────────────────────────────────────

/// Make a non-streaming LLM call.
///
/// Builds the request from session state, then delegates to
/// [`UnifiedFallbackClient::chat`].
pub async fn call_llm(
    unified_fallback_client: &UnifiedFallbackClient,
    content: &str,
    meta: &MessageMetadata,
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<UnifiedResponse, LLMError> {
    let request = build_request(session_manager, session_id, meta, content, false).await?;

    // Acquire this session's cancellation token so an in-flight
    // request can be aborted by a cascade stop.
    let cancel_token: CancellationToken =
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            cs.read().await.cancel_token().clone()
        } else {
            CancellationToken::new()
        };

    tokio::select! {
        res = unified_fallback_client.chat(request) => res,
        _ = cancel_token.cancelled() => {
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.set_llm_state(LlmState::Idle);
            }
            tracing::info!(session_id = %session_id, "LLM request cancelled");
            Err(LLMError::Cancelled)
        }
    }
}

// ── Streaming LLM call ─────────────────────────────────────────────

/// Make a streaming LLM call.
///
/// Builds the request from session state, opens the LLM stream, wraps
/// it with [`SinkUpdater`] for session-level notifications, and returns
/// the stream for the caller to dispatch through the outbound pipeline.
///
/// The caller is responsible for:
/// - Setting LLM state before/after the call
/// - Racing the stream against a cancellation token
/// - Consuming events from the returned stream
pub async fn call_llm_streaming(
    unified_client: &UnifiedChatClient,
    content: &str,
    meta: &MessageMetadata,
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<
    (
        closeclaw_llm::protocol::OutgoingEventStream,
        Option<Arc<dyn StreamingSink>>,
    ),
    LLMError,
> {
    let request = build_request(session_manager, session_id, meta, content, true).await?;

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

    let stream = match unified_client.chat_streaming(request).await {
        Ok(s) => s,
        Err(e) => {
            let msg = e.to_string();
            tracing::error!(error = %msg, "streaming LLM call failed");
            if let Some(ref s) = sink {
                s.send_error(msg.clone());
            }
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.set_llm_state(LlmState::Idle);
            }
            return Err(LLMError::ApiError(msg));
        }
    };

    Ok((stream, sink))
}

// ── SinkUpdater: stream wrapper ────────────────────────────────────

/// Wraps an LLM [`Stream`](futures::Stream) so each event is mirrored
/// to the session's [`StreamingSink`] before being forwarded downstream.
///
/// - `BlockDelta(Text)` → `sink.send_text` (and triggers the
///   `Requesting → Receiving` state transition on the first text delta).
/// - `MessageEnd` → `sink.send_done` with the final usage.
/// - `Error` → `sink.send_error`.
pub struct SinkUpdater<S> {
    inner: S,
    sink: Option<Arc<dyn StreamingSink>>,
    session_manager: Arc<SessionManager>,
    session_id: String,
    first_token: bool,
}

impl<S> SinkUpdater<S> {
    pub fn new(
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

impl<S> futures::Stream for SinkUpdater<S>
where
    S: futures::Stream<
            Item = Result<
                closeclaw_llm::types::StreamEvent,
                closeclaw_llm::protocol::ProtocolError,
            >,
        > + Unpin,
{
    type Item = Result<closeclaw_llm::types::StreamEvent, closeclaw_llm::protocol::ProtocolError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use closeclaw_llm::streaming::StreamDone;
        use closeclaw_llm::types::{ContentDelta, StreamEvent};

        match std::pin::Pin::new(&mut self.inner).poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(event))) => {
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
                std::task::Poll::Ready(Some(Ok(event)))
            }
            std::task::Poll::Ready(Some(Err(e))) => {
                let msg = e.to_string();
                if let Some(ref sink) = self.sink {
                    sink.send_error(msg);
                }
                std::task::Poll::Ready(Some(Err(e)))
            }
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
