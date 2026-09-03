//! Unit tests for Feishu streaming rendering integration.
//!
//! Tests the full pipeline:
//! - `DefaultStreamingRenderer` → `StreamingOutput` via `handle_stream_event`
//! - `CardkitStreamingRenderer` pending_text accumulation
//! - `FeishuPlugin` delegation (`handle_stream_event`, `flush_stream`, `check_stream_timeout`)
//!
//! Does NOT call real Feishu APIs — uses test-only adapter construction.

use super::cardkit_streaming::CardkitStreamingRenderer;
use super::FeishuPlugin;
use crate::plugin::IMPlugin;
use closeclaw_common::processor::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent};
use closeclaw_common::StreamingOutput;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_test_adapter() -> super::adapter::FeishuAdapter {
    let tmp = tempfile::TempDir::new().expect("tmp dir");
    let store = Arc::new(
        crate::media_store::MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"),
    );
    super::adapter::FeishuAdapter::new("test_profile".to_string(), store)
}

fn make_plugin() -> FeishuPlugin {
    let adapter = Arc::new(make_test_adapter());
    FeishuPlugin::new(adapter)
}

fn block_start(index: usize, bt: ContentBlockType) -> StreamEvent {
    StreamEvent::BlockStart {
        index,
        block_type: bt,
    }
}

fn block_end(index: usize, bt: ContentBlockType) -> StreamEvent {
    StreamEvent::BlockEnd {
        index,
        block_type: bt,
    }
}

fn text_delta(index: usize, text: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index,
        delta: ContentDelta::Text {
            text: text.to_string(),
        },
    }
}

fn thinking_delta(index: usize, thinking: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index,
        delta: ContentDelta::Thinking {
            thinking: thinking.to_string(),
            signature: None,
        },
    }
}

fn tool_use_delta_id(index: usize, id: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index,
        delta: ContentDelta::ToolUseId { id: id.to_string() },
    }
}

fn tool_use_delta_name(index: usize, name: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index,
        delta: ContentDelta::ToolUseName {
            name: name.to_string(),
        },
    }
}

fn tool_use_delta_input(index: usize, input: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index,
        delta: ContentDelta::ToolUseInputChunk {
            input: input.to_string(),
        },
    }
}

// ===========================================================================
// Normal path: Text block BlockStart → BlockDelta → BlockEnd
// ===========================================================================

/// Text block with sentence terminator produces completed line via
/// the DefaultStreamingRenderer, routed through FeishuPlugin.
#[test]
fn normal_text_block_sentence_terminator_emits_line() {
    let plugin = make_plugin();
    let out = plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());

    let out = plugin.handle_stream_event(text_delta(0, "Hello world."));
    assert_eq!(out.text_messages, vec!["Hello world."]);
    assert!(out.render_blocks.is_empty());

    let out = plugin.handle_stream_event(block_end(0, ContentBlockType::Text));
    assert!(out.text_messages.is_empty());
}

/// Text block accumulates partial text (no terminator) and flushes at block end.
#[test]
fn normal_text_block_partial_flushes_at_end() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "partial content"));
    assert!(out.text_messages.is_empty());

    let out = plugin.handle_stream_event(block_end(0, ContentBlockType::Text));
    assert_eq!(out.text_messages, vec!["partial content"]);
}

/// Multiple deltas accumulate and emit on sentence terminator.
#[test]
fn normal_text_multiple_deltas_accumulate() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "Hello "));
    assert!(out.text_messages.is_empty());
    let out = plugin.handle_stream_event(text_delta(0, "world."));
    assert_eq!(out.text_messages, vec!["Hello world."]);
}

/// BlockEnd flushes remaining buffered text.
#[test]
fn normal_text_block_end_flushes_remaining() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "some text without terminator"));
    let out = plugin.handle_stream_event(block_end(0, ContentBlockType::Text));
    assert_eq!(out.text_messages, vec!["some text without terminator"]);
}

// ===========================================================================
// Line buffering: sentence terminators, newlines, 100-char threshold
// ===========================================================================

/// Chinese sentence terminators trigger line emission.
#[test]
fn line_buffer_chinese_terminators() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "你好世界！"));
    assert_eq!(out.text_messages, vec!["你好世界！"]);
}

/// Newlines trigger line emission.
#[test]
fn line_buffer_newline_triggers() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "line one\nline two\n"));
    assert_eq!(out.text_messages, vec!["line one\n", "line two\n"]);
}

/// 100-character threshold forces emission.
#[test]
fn line_buffer_threshold_forces_emission() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let long_text = "a".repeat(150);
    let out = plugin.handle_stream_event(text_delta(0, &long_text));
    assert_eq!(out.text_messages.len(), 1);
    assert_eq!(out.text_messages[0].chars().count(), 150);
}

/// English sentence terminators trigger line emission.
#[test]
fn line_buffer_english_terminators() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "Done. Ready?"));
    assert_eq!(out.text_messages, vec!["Done.", " Ready?"]);
}

// ===========================================================================
// Code block processing: ``` boundary markers switch code/text mode
// ===========================================================================

/// Code block fence opens and closes code mode; inner periods don't split.
#[test]
fn code_block_boundaries_preserve_content() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "```\nfoo.bar.baz\n```\n"));
    assert_eq!(out.text_messages, vec!["```\n", "foo.bar.baz\n", "```\n"]);
}

/// Code block with language hint opens code mode.
#[test]
fn code_block_with_language_hint() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "```rust\nfn main() {}\n```\n"));
    assert_eq!(
        out.text_messages,
        vec!["```rust\n", "fn main() {}\n", "```\n"]
    );
}

/// Code block content exceeding 100-char threshold still emits in LineByLine mode.
#[test]
fn code_block_threshold_emits_in_line_by_line() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "```\n"));
    let long_code = "x".repeat(120);
    let out = plugin.handle_stream_event(text_delta(0, &format!("{}\n", long_code)));
    assert_eq!(out.text_messages.len(), 1);
    assert!(out.text_messages[0].contains(&long_code));
}

/// Text after code block resumes sentence terminator splitting.
#[test]
fn text_after_code_block_resumes_splitting() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "```\ncode\n```\n"));
    let out = plugin.handle_stream_event(text_delta(0, "Back to text. Done."));
    assert_eq!(out.text_messages, vec!["Back to text.", " Done."]);
}

// ===========================================================================
// Multi-block scenarios: multiple Text blocks processed independently
// ===========================================================================

/// Two sequential Text blocks are independent; second block starts fresh.
#[test]
fn multi_block_two_text_blocks_independent() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "Block one."));
    // "Block one." ends with sentence terminator '.' → emitted during text_delta
    let out = plugin.handle_stream_event(block_end(0, ContentBlockType::Text));
    assert!(out.text_messages.is_empty());

    plugin.handle_stream_event(block_start(1, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(1, "Block two."));
    // "Block two." also emitted during text_delta due to sentence terminator
    let out = plugin.handle_stream_event(block_end(1, ContentBlockType::Text));
    assert!(out.text_messages.is_empty());
}

/// Three Text blocks each with partial content, flushed at block end.
#[test]
fn multi_block_three_text_blocks_partial() {
    let plugin = make_plugin();

    for (i, text) in ["first", "second", "third"].iter().enumerate() {
        plugin.handle_stream_event(block_start(i, ContentBlockType::Text));
        plugin.handle_stream_event(text_delta(i, text));
        let out = plugin.handle_stream_event(block_end(i, ContentBlockType::Text));
        assert_eq!(out.text_messages.len(), 1);
        assert_eq!(out.text_messages[0], *text);
    }
}

// ===========================================================================
// Thinking / Tool blocks: render_blocks returned, not discarded
// ===========================================================================

/// Thinking block produces render_blocks (not discarded).
#[test]
fn thinking_block_emits_render_blocks() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Thinking));
    plugin.handle_stream_event(thinking_delta(0, "Let me think..."));
    let out = plugin.handle_stream_event(block_end(0, ContentBlockType::Thinking));
    assert!(out.text_messages.is_empty());
    assert_eq!(
        out.render_blocks,
        vec![ContentBlock::Thinking {
            thinking: "Let me think...".to_string(),
            signature: None,
        }]
    );
}

/// ToolUse block produces render_blocks with id, name, and input.
#[test]
fn tool_use_block_emits_render_blocks() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::ToolUse));
    plugin.handle_stream_event(tool_use_delta_id(0, "call_123"));
    plugin.handle_stream_event(tool_use_delta_name(0, "search"));
    plugin.handle_stream_event(tool_use_delta_input(0, r#"{"q":"rust"}"#));
    let out = plugin.handle_stream_event(block_end(0, ContentBlockType::ToolUse));
    assert!(out.text_messages.is_empty());
    assert_eq!(
        out.render_blocks,
        vec![ContentBlock::ToolUse {
            id: "call_123".to_string(),
            name: "search".to_string(),
            input: r#"{"q":"rust"}"#.to_string(),
        }]
    );
}

/// Thinking block interleaved with Text blocks: both outputs are independent.
#[test]
fn thinking_interleaved_with_text() {
    let plugin = make_plugin();

    // Text block
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "Answer."));
    assert_eq!(out.text_messages, vec!["Answer."]);
    plugin.handle_stream_event(block_end(0, ContentBlockType::Text));

    // Thinking block
    plugin.handle_stream_event(block_start(1, ContentBlockType::Thinking));
    plugin.handle_stream_event(thinking_delta(1, "reasoning"));
    let out = plugin.handle_stream_event(block_end(1, ContentBlockType::Thinking));
    assert!(out.text_messages.is_empty());
    assert_eq!(out.render_blocks.len(), 1);

    // Another Text block
    plugin.handle_stream_event(block_start(2, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(2, "More text."));
    assert_eq!(out.text_messages, vec!["More text."]);
    assert!(out.render_blocks.is_empty());
}

// ===========================================================================
// Timeout: check_stream_timeout delegates to DefaultStreamingRenderer
// ===========================================================================

/// check_stream_timeout returns empty when nothing buffered.
#[test]
fn check_stream_timeout_empty_returns_empty() {
    let plugin = make_plugin();
    let out = plugin.check_stream_timeout();
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());
}

/// check_stream_timeout returns buffered text after timeout elapses.
#[test]
fn check_stream_timeout_returns_buffered_text() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "buffered"));
    std::thread::sleep(std::time::Duration::from_millis(250));
    let out = plugin.check_stream_timeout();
    assert!(
        !out.text_messages.is_empty(),
        "Expected text from timeout, got: {:?}",
        out
    );
    assert!(out.text_messages.iter().any(|m| m.contains("buffered")));
}

// ===========================================================================
// MessageEnd: flush_stream flushes all buffers
// ===========================================================================

/// flush_stream returns remaining text from open Text block.
#[test]
fn flush_stream_returns_remaining_text() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "partial"));
    let out = plugin.flush_stream();
    assert_eq!(out.text_messages, vec!["partial"]);
}

/// flush_stream returns empty when all buffers already drained.
#[test]
fn flush_stream_returns_empty_when_drained() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "done."));
    plugin.handle_stream_event(block_end(0, ContentBlockType::Text));
    let out = plugin.flush_stream();
    assert!(out.text_messages.is_empty());
}

/// flush_stream resets block state for next message.
#[test]
fn flush_stream_resets_block_state() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    plugin.handle_stream_event(text_delta(0, "partial"));
    let out = plugin.flush_stream();
    assert_eq!(out.text_messages, vec!["partial"]);

    // After flush, new block should start fresh.
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "Fresh start."));
    assert_eq!(out.text_messages, vec!["Fresh start."]);
    assert!(out.render_blocks.is_empty());
}

/// flush_stream returns empty on completely empty state.
#[test]
fn flush_stream_empty_state_returns_empty() {
    let plugin = make_plugin();
    let out = plugin.flush_stream();
    assert_eq!(out, StreamingOutput::default());
}

// ===========================================================================
// Cardkit update frequency: 100ms interval limit
// ===========================================================================

/// CardkitStreamingRenderer respects 100ms update interval.
#[test]
fn cardkit_update_frequency_100ms_interval() {
    let mut renderer = CardkitStreamingRenderer::new();
    assert!(
        renderer.should_update_now(),
        "First update should be allowed"
    );

    // Simulate recording an update.
    renderer.state.last_update = Some(std::time::Instant::now());
    // Immediately after: should not update (within 100ms).
    assert!(
        !renderer.should_update_now(),
        "Should not update within 100ms"
    );
}

/// CardkitStreamingRenderer allows update after 100ms interval.
#[test]
fn cardkit_allows_update_after_100ms() {
    let mut renderer = CardkitStreamingRenderer::new();
    renderer.state.last_update =
        Some(std::time::Instant::now() - std::time::Duration::from_millis(150));
    assert!(
        renderer.should_update_now(),
        "Should allow update after 150ms"
    );
}

/// CardkitStreamingRenderer blocks update at exactly 99ms.
#[test]
fn cardkit_blocks_update_at_99ms() {
    let mut renderer = CardkitStreamingRenderer::new();
    renderer.state.last_update =
        Some(std::time::Instant::now() - std::time::Duration::from_millis(99));
    assert!(
        !renderer.should_update_now(),
        "Should not allow update at 99ms"
    );
}

// ===========================================================================
// Cardkit text accumulation: pending_text populated from handle_stream_event
// ===========================================================================

/// Cardkit pending_text accumulates text from handle_stream_event output.
#[test]
fn cardkit_accumulates_text_from_stream_output() {
    let plugin = make_plugin();
    // Feed text that triggers line emission.
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, "Hello world."));
    assert_eq!(out.text_messages, vec!["Hello world."]);

    // Simulate what the gateway does: accumulate text into cardkit pending_text.
    // This is the accumulate_streaming_text path.
    let mut cardkit = plugin.cardkit_streaming.lock().unwrap();
    for msg in &out.text_messages {
        cardkit.state.pending_text.push_str(msg);
    }
    assert_eq!(cardkit.state.pending_text, "Hello world.");
}

/// Cardkit pending_text is empty after flush.
#[test]
fn cardkit_pending_text_empty_initially() {
    let plugin = make_plugin();
    assert!(plugin.cardkit_pending_text().is_empty());
}

// ===========================================================================
// Empty input: empty text and events produce no output
// ===========================================================================

/// Empty text delta produces no output.
#[test]
fn empty_text_delta_produces_no_output() {
    let plugin = make_plugin();
    plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    let out = plugin.handle_stream_event(text_delta(0, ""));
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());
}

/// MessageEnd event produces no output.
#[test]
fn message_end_event_produces_no_output() {
    let plugin = make_plugin();
    let out = plugin.handle_stream_event(StreamEvent::MessageEnd {
        usage: None,
        finish_reason: None,
    });
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());
}

/// Error event produces no output.
#[test]
fn error_event_produces_no_output() {
    let plugin = make_plugin();
    let out = plugin.handle_stream_event(StreamEvent::Error {
        message: "test error".to_string(),
    });
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());
}

/// BlockStart alone produces no output.
#[test]
fn block_start_alone_produces_no_output() {
    let plugin = make_plugin();
    let out = plugin.handle_stream_event(block_start(0, ContentBlockType::Text));
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());
}

/// Handle stream event with no prior state produces empty output.
#[test]
fn no_prior_state_produces_empty_output() {
    let plugin = make_plugin();
    let out = plugin.check_stream_timeout();
    assert_eq!(out, StreamingOutput::default());
    let out = plugin.flush_stream();
    assert_eq!(out, StreamingOutput::default());
}
