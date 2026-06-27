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
    _session_manager: &Arc<SessionManager>,
    _session_id: &str,
) -> Result<UnifiedResponse, LLMError> {
    let request = InternalRequest {
        model: String::new(), // fallback client picks the model
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: content.to_string(),
        }],
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
    _session_manager: &Arc<SessionManager>,
    _session_id: &str,
) -> Result<
    (
        Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
        Option<Arc<dyn StreamingSink>>,
    ),
    LLMError,
> {
    let request = InternalRequest {
        model: String::new(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: content.to_string(),
        }],
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

/// Execute a compaction: call the LLM to summarize the conversation,
/// return the compaction result with the boundary message.
pub async fn execute_compact(
    messages: &[closeclaw_llm::Message],
    client: &closeclaw_llm::fallback::FallbackClient,
    model: &str,
    instruction: Option<&str>,
    is_auto: bool,
) -> Result<
    closeclaw_session::compaction::CompactionResult,
    closeclaw_session::compaction::CompactionError,
> {
    use closeclaw_llm::{ChatRequest, Message as LlmMessage};
    use closeclaw_session::compaction::*;

    if messages.is_empty() {
        return Err(CompactionError::EmptyMessages);
    }

    let prompt = build_compact_prompt(instruction);
    let mut llm_messages = vec![LlmMessage {
        role: "system".to_string(),
        content: prompt,
    }];
    for m in messages {
        llm_messages.push(LlmMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }

    let request = ChatRequest {
        model: model.to_string(),
        messages: llm_messages,
        temperature: 0.0,
        max_tokens: Some(4096),
    };

    let response = client
        .chat(request)
        .await
        .map_err(|e| CompactionError::LLMCallFailed(e.to_string()))?;

    let summary = extract_summary(&response.content).ok_or(CompactionError::SummaryParseFailed)?;

    let boundary = format_boundary_message(&summary, is_auto);
    let before_tokens = estimate_messages_tokens(
        &messages
            .iter()
            .map(|m| CompactionMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect::<Vec<_>>(),
    );
    let after_tokens = estimate_tokens(&boundary);
    let before_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    let after_chars = boundary.len();

    Ok(CompactionResult {
        performed: true,
        original_tokens: before_tokens,
        compacted_tokens: after_tokens,
        message: format!(
            "Compaction completed: {} → {} tokens",
            before_tokens, after_tokens
        ),
        before_char_count: before_chars,
        after_char_count: after_chars,
        before_token_count: before_tokens,
        after_token_count: after_tokens,
        boundary_message: boundary,
        is_auto,
    })
}
