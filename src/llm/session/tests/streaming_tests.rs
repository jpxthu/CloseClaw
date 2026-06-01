// ── Streaming integration tests ─────────────────────────────────────────────

use super::*;
use std::path::PathBuf;

#[test]
fn test_conversation_session_stream_defaults() {
    let session = ConversationSession::new("s1".into(), "m".into(), PathBuf::from("/tmp"));
    assert!(!session.stream_enabled());
    assert!(session.streaming_sink().is_none());
}

#[test]
fn test_set_stream_enabled() {
    let mut session = ConversationSession::new("s1".into(), "m".into(), PathBuf::from("/tmp"));
    assert!(!session.stream_enabled());
    session.set_stream_enabled(true);
    assert!(session.stream_enabled());
    session.set_stream_enabled(false);
    assert!(!session.stream_enabled());
}

#[test]
fn test_build_api_request_stream_flag_reflects_state() {
    let mut session = ConversationSession::new("s1".into(), "m".into(), PathBuf::from("/tmp"));
    // Default: stream = false
    let req = session.build_api_request();
    assert!(!req.stream);

    // Enable streaming
    session.set_stream_enabled(true);
    let req = session.build_api_request();
    assert!(req.stream);

    // Disable streaming
    session.set_stream_enabled(false);
    let req = session.build_api_request();
    assert!(!req.stream);
}

#[test]
fn test_set_streaming_sink() {
    use crate::llm::streaming::{StreamDone, StreamingSink};
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct MockSink {
        texts: Mutex<Vec<String>>,
    }
    impl MockSink {
        fn new() -> Self {
            Self {
                texts: Mutex::new(Vec::new()),
            }
        }
    }
    impl StreamingSink for MockSink {
        fn send_text(&self, delta: &str) {
            self.texts.lock().unwrap().push(delta.to_string());
        }
        fn send_done(&self, _done: StreamDone) {}
        fn send_error(&self, _error: String) {}
    }

    let mut session = ConversationSession::new("s1".into(), "m".into(), PathBuf::from("/tmp"));
    assert!(session.streaming_sink().is_none());

    let sink: Arc<dyn StreamingSink> = Arc::new(MockSink::new());
    session.set_streaming_sink(sink);
    assert!(session.streaming_sink().is_some());

    // Verify sink works through session
    session.streaming_sink().unwrap().send_text("hello");
}
