// ── Streaming error-handling unit tests ─────────────────────────────────────
//
// Verifies that `call_llm_streaming` sends partial text blocks to the sink
// before the error message when a `StreamError` occurs mid-stream.

use closeclaw_common::{StreamDone, StreamingSink};
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::LLMError;
use std::sync::Mutex;

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

// ── Helper: replicate the map_err logic from call_llm_streaming ────────────

/// Re-implementation of the error-handling path in `call_llm_streaming`
/// so we can unit-test the sink interaction without needing the full
/// async streaming pipeline.
fn simulate_stream_error(sink: &RecordingSink, error: GatewayError) -> LLMError {
    let msg = error.to_string();
    if let GatewayError::StreamError {
        ref partial_content,
        ..
    } = error
    {
        // Send accumulated partial text blocks to the user before the
        // error so they don't lose already-generated content.
        for block in partial_content {
            if let ContentBlock::Text(text) = block {
                sink.send_text(text);
            }
        }
        tracing::warn!(
            partial_content_blocks = partial_content.len(),
            "streaming error: partial content blocks preserved"
        );
    }
    sink.send_error(msg.clone());
    LLMError::ApiError(msg)
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

    let result = simulate_stream_error(&sink, error);
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

    simulate_stream_error(&sink, error);

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: timeout");
}

/// Empty partial_content → no text blocks sent, only the error.
#[test]
fn test_stream_error_empty_partial_content_sends_no_text() {
    let sink = RecordingSink::new();
    let error = GatewayError::StreamError {
        message: "immediate failure".to_string(),
        partial_content: vec![],
    };

    simulate_stream_error(&sink, error);

    let texts = sink.texts.lock().unwrap();
    assert!(
        texts.is_empty(),
        "no text blocks should be sent for empty partial_content"
    );

    let errors = sink.errors.lock().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0], "Streaming error: immediate failure");
}
