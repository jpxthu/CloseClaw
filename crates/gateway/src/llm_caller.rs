//! LLM caller helpers for the gateway layer.
//!
//! Provides thin wrappers around the LLM client for non-streaming and
//! streaming calls, plus a [`SinkUpdater`] adapter that forwards stream
//! events to a [`StreamingSink`].

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};

use closeclaw_llm::client::UnifiedChatClient;
use closeclaw_llm::session::{InjectionPosition, MemoryInjection};
use closeclaw_llm::streaming::{StreamDone, StreamingSink};
use closeclaw_llm::types::{
    ContentDelta, InternalMessage, InternalRequest, StreamEvent, UnifiedResponse,
};
use closeclaw_llm::LLMError;

use crate::session_manager::SessionManager;
use closeclaw_session::persistence::ReasoningLevel;

/// Make a non-streaming LLM call.
///
/// Builds an [`InternalRequest`] from the content and delegates to the
/// unified fallback client. Returns the [`UnifiedResponse`].
pub async fn call_llm(
    client: &closeclaw_llm::unified_fallback::UnifiedFallbackClient,
    content: &str,
    _meta: &crate::session_handler::MessageMetadata,
    session_manager: &Arc<SessionManager>,
    session_id: &str,
) -> Result<UnifiedResponse, LLMError> {
    let mut messages = vec![InternalMessage {
        role: "user".to_string(),
        content: content.to_string(),
        tool_call_id: None,
    }];

    // Consume memory_injection slot if present.
    if let Some(injection) = consume_memory_injection(session_manager, session_id).await {
        let tool_msg = memory_injection_to_message(&injection);
        match injection.position_mode {
            InjectionPosition::AfterCurrent => {
                messages.push(tool_msg);
            }
            InjectionPosition::BeforeNext => {
                messages.insert(0, tool_msg);
            }
        }
    }

    let request = InternalRequest {
        model: String::new(), // fallback client picks the model
        messages,
        temperature: 0.7,
        max_tokens: None,
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    };
    client.chat(request).await
}

/// Make a streaming LLM call.
///
/// Returns a tuple of `(event_stream, optional_sink)` where the event
/// stream yields [`StreamEvent`] values and the sink is the session's
/// streaming sink (if any) for incremental text notifications.
pub async fn call_llm_streaming(
    client: &UnifiedChatClient,
    content: &str,
    _meta: &crate::session_handler::MessageMetadata,
    session_manager: &Arc<SessionManager>,
    session_id: &str,
) -> Result<
    (
        Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
        Option<Arc<dyn StreamingSink>>,
    ),
    LLMError,
> {
    let mut messages = vec![InternalMessage {
        role: "user".to_string(),
        content: content.to_string(),
        tool_call_id: None,
    }];

    // Consume memory_injection slot if present.
    if let Some(injection) = consume_memory_injection(session_manager, session_id).await {
        let tool_msg = memory_injection_to_message(&injection);
        match injection.position_mode {
            InjectionPosition::AfterCurrent => {
                messages.push(tool_msg);
            }
            InjectionPosition::BeforeNext => {
                messages.insert(0, tool_msg);
            }
        }
    }

    let request = InternalRequest {
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
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    };
    let raw_stream = client.chat_streaming(request).await?;
    // Map the raw event stream into the expected Item type.
    let mapped = raw_stream.map(
        |r: Result<StreamEvent, closeclaw_llm::protocol::ProtocolError>| {
            r.map_err(|e| LLMError::ApiError(e.to_string()))
        },
    );
    Ok((Box::pin(mapped), None))
}

/// Consume the `memory_injection` slot from the given session.
///
/// Returns the injection if the session exists and the slot was populated.
/// The slot is cleared (one-shot consumption) regardless of the result.
pub(crate) async fn consume_memory_injection(
    session_manager: &Arc<SessionManager>,
    session_id: &str,
) -> Option<MemoryInjection> {
    let cs = session_manager.get_conversation_session(session_id).await?;
    let cs = cs.read().await;
    cs.take_memory_injection()
}

/// Convert a [`MemoryInjection`] into a tool-role [`InternalMessage`].
pub(crate) fn memory_injection_to_message(injection: &MemoryInjection) -> InternalMessage {
    InternalMessage {
        role: "tool".to_string(),
        content: injection.content.clone(),
        tool_call_id: None,
    }
}

/// Stream adapter that forwards text deltas to a [`StreamingSink`].
///
/// Wraps an inner event stream and forwards [`StreamEvent::BlockDelta`]
/// text deltas to the sink while passing events through unchanged.
pub struct SinkUpdater<S> {
    inner: S,
    sink: Option<Arc<dyn StreamingSink>>,
}

impl<S> SinkUpdater<S> {
    /// Create a new `SinkUpdater` wrapping the given stream and sink.
    pub fn new(
        inner: S,
        sink: Option<Arc<dyn StreamingSink>>,
        _session_manager: Arc<SessionManager>,
        _session_id: String,
    ) -> Self {
        Self { inner, sink }
    }
}

impl<S, E> Stream for SinkUpdater<S>
where
    S: Stream<Item = Result<StreamEvent, E>> + Unpin,
    E: std::fmt::Display,
{
    type Item = Result<StreamEvent, LLMError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                if let Some(ref sink) = this.sink {
                    match &event {
                        StreamEvent::BlockDelta {
                            delta: ContentDelta::Text { text },
                            ..
                        } => {
                            sink.send_text(text);
                        }
                        StreamEvent::MessageEnd { usage, .. } => {
                            sink.send_done(StreamDone {
                                model: String::new(),
                                usage: usage.clone(),
                            });
                        }
                        StreamEvent::Error { message } => {
                            sink.send_error(message.clone());
                        }
                        _ => {}
                    }
                }
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(Some(Err(e))) => {
                if let Some(ref sink) = this.sink {
                    sink.send_error(e.to_string());
                }
                Poll::Ready(Some(Err(LLMError::ApiError(e.to_string()))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
