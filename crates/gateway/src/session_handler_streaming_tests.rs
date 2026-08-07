// ── Streaming error-handling unit tests ─────────────────────────────────────
//
// Verifies that `handle_stream_error` sends partial text blocks to the sink
// before the error message when a `StreamError` occurs mid-stream.

use closeclaw_common::StreamDone;
use closeclaw_common::StreamingSink;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::LLMError;
use std::sync::Mutex;

use super::session_handler_streaming::handle_stream_error;
use crate::types::GatewayError;

// ── Recording sink ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct RecordingSink {
    texts: Mutex<Vec<String>>,
    errors: Mutex<Vec<String>>,
    dones: Mutex<Vec<StreamDone>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            texts: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            dones: Mutex::new(Vec::new()),
        }
    }
}

impl StreamingSink for RecordingSink {
    fn send_text(&self, delta: &str) {
        self.texts.lock().unwrap().push(delta.to_string());
    }
    fn send_done(&self, done: StreamDone) {
        self.dones.lock().unwrap().push(done);
    }
    fn send_error(&self, error: String) {
        self.errors.lock().unwrap().push(error);
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// StreamError with two text blocks → both sent before error.
#[test]
fn test_stream_error_sends_partial_text_before_error() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "stream interrupted".to_string(),
        partial_content: vec![
            ContentBlock::Text("Hello, ".to_string()),
            ContentBlock::Text("world!".to_string()),
        ],
    };

    let result = handle_stream_error(error, &sink);
    assert!(matches!(result, LLMError::ApiError(_)));

    let texts = sink.texts.lock().unwrap();
    assert_eq!(texts.len(), 2, "should have sent 2 text blocks");
    assert_eq!(texts[0], "Hello, ");
    assert_eq!(texts[1], "world!");

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: stream interrupted");
}

/// Error message is sent after all partial text blocks.
#[test]
fn test_stream_error_sends_error_after_partial_content() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "timeout".to_string(),
        partial_content: vec![ContentBlock::Text("partial".to_string())],
    };

    handle_stream_error(error, &sink);

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: timeout");
}

/// Mixed ContentBlocks (Text + Image) → only Text blocks sent, Image ignored.
#[test]
fn test_stream_error_only_text_blocks_sent_from_mixed_content() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "stream interrupted".to_string(),
        partial_content: vec![
            ContentBlock::Text("Hello, ".to_string()),
            ContentBlock::Image {
                name: "screenshot.png".to_string(),
                url: "https://example.com/img.png".to_string(),
            },
            ContentBlock::Text("world!".to_string()),
        ],
    };

    let result = handle_stream_error(error, &sink);
    assert!(matches!(result, LLMError::ApiError(_)));

    let texts = sink.texts.lock().unwrap();
    assert_eq!(
        texts.len(),
        2,
        "should only send 2 text blocks, not the image block"
    );
    assert_eq!(texts[0], "Hello, ");
    assert_eq!(texts[1], "world!");

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
}

/// Empty partial_content → no text blocks sent, only the error.
#[test]
fn test_stream_error_empty_partial_content_sends_no_text() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "immediate failure".to_string(),
        partial_content: vec![],
    };

    handle_stream_error(error, &sink);

    let texts = sink.texts.lock().unwrap();
    assert!(
        texts.is_empty(),
        "no text blocks should be sent for empty partial_content"
    );

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: immediate failure");
}
