//! Platform-agnostic streaming sink for LLM output.
//!
//! Session layer holds a [`StreamingSink`] handle and pushes incremental
//! text deltas, completion notifications (carrying model + usage), and
//! error notifications as the underlying [`crate::llm::UnifiedChatClient`]
//! produces [`crate::llm::StreamEvent`]s. Implementations forward these to
//! a downstream transport (Feishu card update, CLI stdout, etc.) without
//! the session needing to know which one is in use.
//!
//! See `docs/design/session/llm-session-enhancements.md` for the full
//! streaming architecture rationale.

use serde::{Deserialize, Serialize};

use crate::llm::types::UnifiedUsage;

/// Notification emitted by a streaming LLM call when the stream completes
/// successfully.
///
/// Carries the model name that produced the final response and the
/// provider-reported token usage (when available). `UnifiedUsage` is
/// `Option` because some providers omit usage on streaming responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamDone {
    /// Model name that produced the final response.
    pub model: String,
    /// Token usage statistics reported by the provider, if available.
    pub usage: Option<UnifiedUsage>,
}

impl StreamDone {
    /// Creates a new `StreamDone` notification.
    pub fn new(model: impl Into<String>, usage: Option<UnifiedUsage>) -> Self {
        Self {
            model: model.into(),
            usage,
        }
    }
}

/// Platform-agnostic sink for streaming LLM output.
///
/// Implementations forward incremental text deltas, completion
/// notifications (with model + usage), and error notifications to a
/// downstream transport. The session layer drives this trait as
/// [`crate::llm::StreamEvent`]s arrive; it never inspects the concrete
/// transport.
///
/// ## Contract
///
/// - `send_text` is called once per textual delta. Implementations must
///   be non-blocking; long-running work should be enqueued internally.
/// - `send_done` is called **exactly once** at the end of a successful
///   stream, after the last `send_text`. No further `send_text` calls
///   follow on the same stream.
/// - `send_error` is called **at most once** per stream invocation when
///   the provider reports an error or the underlying connection is
///   unrecoverable. No further `send_text` / `send_done` calls follow on
///   the same stream.
///
/// ## Threading
///
/// `Send + Sync` is required so the sink can be shared across async
/// tasks and held in `Arc<dyn StreamingSink>`.
pub trait StreamingSink: Send + Sync {
    /// Push a single text delta (incremental content fragment).
    fn send_text(&self, delta: &str);

    /// Notify that the stream completed successfully.
    fn send_done(&self, done: StreamDone);

    /// Notify that the stream failed.
    fn send_error(&self, error: String);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal no-op sink used to verify trait usability.
    struct NoopSink {
        text: std::sync::Mutex<String>,
    }

    impl StreamingSink for NoopSink {
        fn send_text(&self, delta: &str) {
            let mut buf = self.text.lock().unwrap();
            buf.push_str(delta);
        }

        fn send_done(&self, _done: StreamDone) {}

        fn send_error(&self, _error: String) {}
    }

    #[test]
    fn test_stream_done_construction_and_equality() {
        let usage = UnifiedUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: Some(30),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        let a = StreamDone::new("gpt-4o", Some(usage.clone()));
        let b = StreamDone {
            model: "gpt-4o".to_string(),
            usage: Some(usage),
        };
        assert_eq!(a, b);
        assert_eq!(a.model, "gpt-4o");
        assert!(a.usage.is_some());
    }

    #[test]
    fn test_stream_done_without_usage() {
        let done = StreamDone::new("claude-opus-4", None);
        assert_eq!(done.model, "claude-opus-4");
        assert!(done.usage.is_none());
    }

    #[test]
    fn test_noop_sink_collects_text() {
        let sink = NoopSink {
            text: std::sync::Mutex::new(String::new()),
        };
        sink.send_text("hello ");
        sink.send_text("world");
        assert_eq!(*sink.text.lock().unwrap(), "hello world");
    }

    #[test]
    fn test_sink_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoopSink>();
    }
}
