//! LLM caller helpers for the session layer.
//!
//! Provides thin wrappers around the LLM client for non-streaming and
//! streaming calls, plus a [`SinkUpdater`] adapter that forwards stream
//! events to a [`StreamingSink`].

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use tokio::sync::RwLock;

use crate::client::UnifiedChatClient;
use crate::streaming::{StreamDone, StreamingSink};
use crate::types::{ContentDelta, InternalMessage, InternalRequest, StreamEvent, UnifiedResponse};
use crate::LLMError;

use closeclaw_session::persistence::ReasoningLevel;

/// Trait for extracting metadata needed by LLM calls without depending
/// on the gateway crate's `MessageMetadata`.
pub trait LlmMeta: Send + Sync {
    fn sender_id(&self) -> &str;
    fn channel(&self) -> &str;
    fn timestamp(&self) -> i64;
}

/// Proxy trait for session manager operations needed by the LLM caller.
/// Avoids a direct dependency on the gateway crate.
#[async_trait::async_trait]
pub trait SessionManagerOps: Send + Sync {
    async fn get_conversation_session(
        &self,
        session_id: &str,
    ) -> Option<Arc<RwLock<crate::session::ConversationSession>>>;
}

/// Make a non-streaming LLM call.
///
/// Builds an [`InternalRequest`] from the content and delegates to the
/// unified fallback client. Returns the [`UnifiedResponse`].
pub async fn call_llm<M: LlmMeta>(
    client: &crate::unified_fallback::UnifiedFallbackClient,
    content: &str,
    _meta: &M,
    _session_manager: &Arc<dyn SessionManagerOps>,
    _session_id: &str,
) -> Result<UnifiedResponse, LLMError> {
    let request = InternalRequest {
        model: String::new(),
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
pub async fn call_llm_streaming<M: LlmMeta>(
    client: &UnifiedChatClient,
    content: &str,
    _meta: &M,
    _session_manager: &Arc<dyn SessionManagerOps>,
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
    let mapped = raw_stream.map(|r| r.map_err(|e| LLMError::ApiError(e.to_string())));
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
        _session_manager: Arc<RwLock<()>>,
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
