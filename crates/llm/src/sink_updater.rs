//! Stream adapter that forwards text deltas to a [`StreamingSink`].
//!
//! Wraps an inner event stream and forwards [`StreamEvent::BlockDelta`]
//! text deltas to the sink while passing events through unchanged.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;

use crate::streaming::{StreamDone, StreamingSink};
use crate::types::{ContentDelta, StreamEvent};
use crate::LLMError;

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
    pub fn new(inner: S, sink: Option<Arc<dyn StreamingSink>>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentBlockType, UnifiedUsage};
    use futures::stream;

    struct TestSink {
        texts: std::sync::Mutex<Vec<String>>,
        done: std::sync::Mutex<Option<StreamDone>>,
        errors: std::sync::Mutex<Vec<String>>,
    }

    impl TestSink {
        fn new() -> Self {
            Self {
                texts: std::sync::Mutex::new(Vec::new()),
                done: std::sync::Mutex::new(None),
                errors: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl StreamingSink for TestSink {
        fn send_text(&self, delta: &str) {
            self.texts.lock().unwrap().push(delta.to_string());
        }
        fn send_done(&self, done: StreamDone) {
            *self.done.lock().unwrap() = Some(done);
        }
        fn send_error(&self, error: String) {
            self.errors.lock().unwrap().push(error);
        }
    }

    #[tokio::test]
    async fn test_text_delta_forwarded_to_sink() {
        let sink = Arc::new(TestSink::new());
        let events: Vec<Result<StreamEvent, LLMError>> = vec![
            Ok(StreamEvent::BlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
            Ok(StreamEvent::BlockDelta {
                index: 0,
                delta: ContentDelta::Text {
                    text: "hello".to_string(),
                },
            }),
            Ok(StreamEvent::BlockDelta {
                index: 0,
                delta: ContentDelta::Text {
                    text: " world".to_string(),
                },
            }),
            Ok(StreamEvent::BlockEnd {
                index: 0,
                block_type: ContentBlockType::Text,
            }),
        ];
        let stream = stream::iter(events);
        let mut updater = SinkUpdater::new(stream, Some(sink.clone()));

        use futures::StreamExt;
        while let Some(item) = updater.next().await {
            let _ = item;
        }

        let texts = sink.texts.lock().unwrap();
        assert_eq!(*texts, vec!["hello", " world"]);
    }

    #[tokio::test]
    async fn test_message_end_forwarded_to_sink() {
        let sink = Arc::new(TestSink::new());
        let usage = UnifiedUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: Some(30),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        let events: Vec<Result<StreamEvent, LLMError>> = vec![Ok(StreamEvent::MessageEnd {
            usage: Some(usage),
            finish_reason: Some("stop".to_string()),
        })];
        let stream = stream::iter(events);
        let mut updater = SinkUpdater::new(stream, Some(sink.clone()));

        use futures::StreamExt;
        while let Some(item) = updater.next().await {
            let _ = item;
        }

        let done = sink.done.lock().unwrap();
        assert!(done.is_some());
    }

    #[tokio::test]
    async fn test_error_event_forwarded_to_sink() {
        let sink = Arc::new(TestSink::new());
        let events: Vec<Result<StreamEvent, LLMError>> = vec![Ok(StreamEvent::Error {
            message: "something went wrong".to_string(),
        })];
        let stream = stream::iter(events);
        let mut updater = SinkUpdater::new(stream, Some(sink.clone()));

        use futures::StreamExt;
        while let Some(item) = updater.next().await {
            let _ = item;
        }

        let errors = sink.errors.lock().unwrap();
        assert_eq!(*errors, vec!["something went wrong"]);
    }

    #[tokio::test]
    async fn test_no_sink_still_passes_events() {
        let events: Vec<Result<StreamEvent, LLMError>> = vec![Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "test".to_string(),
            },
        })];
        let stream = stream::iter(events);
        let mut updater = SinkUpdater::new(stream, None);

        use futures::StreamExt;
        let item = updater.next().await;
        assert!(item.is_some());
        assert!(item.unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_inner_error_propagated() {
        let events: Vec<Result<StreamEvent, LLMError>> =
            vec![Err(LLMError::ApiError("inner error".to_string()))];
        let stream = stream::iter(events);
        let mut updater = SinkUpdater::new(stream, None);

        use futures::StreamExt;
        let item = updater.next().await;
        assert!(item.is_some());
        assert!(item.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_inner_error_with_sink_sends_error() {
        let sink = Arc::new(TestSink::new());
        let events: Vec<Result<StreamEvent, LLMError>> =
            vec![Err(LLMError::ApiError("stream error".to_string()))];
        let stream = stream::iter(events);
        let mut updater = SinkUpdater::new(stream, Some(sink.clone()));

        use futures::StreamExt;
        while let Some(item) = updater.next().await {
            let _ = item;
        }

        let errors = sink.errors.lock().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("stream error"));
    }

    #[test]
    fn test_sink_updater_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SinkUpdater<futures::stream::Empty<Result<StreamEvent, LLMError>>>>();
    }
}
