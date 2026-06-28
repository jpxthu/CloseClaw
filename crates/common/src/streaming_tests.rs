//! Unit tests for [`super::streaming`].
//!
//! Covers LineBuffer splitting/flushing, DefaultStreamingRenderer
//! incremental rendering, code block detection via triple backticks,
//! and state reset on flush/block_end.

use super::streaming::*;
use crate::processor::{ContentBlockType, ContentDelta, StreamEvent};

fn text_delta(text: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Text {
            text: text.to_string(),
        },
    }
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

/// Helper: create renderer, start a text block, feed all chunks, flush, and
/// return all collected text messages.
fn feed_chunks(chunks: &[&str]) -> Vec<String> {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    let mut out = Vec::new();
    for c in chunks {
        let result = r.handle_event(text_delta(c));
        out.extend(result.text_messages);
    }
    let flushed = r.flush();
    out.extend(flushed.text_messages);
    out
}

// ── Normal text splitting ───────────────────────────────────────────────────

#[test]
fn test_text_splits_on_sentence_terminators() {
    let out = feed_chunks(&["Hello world. 你好世界!\nDone?"]);
    assert_eq!(out, vec!["Hello world.", "你好世界!", "Done?"]);
}

#[test]
fn test_text_splits_on_newline() {
    let out = feed_chunks(&["line1\nline2\n"]);
    assert_eq!(out, vec!["line1", "line2"]);
}

#[test]
fn test_partial_terminator_buffered_until_complete() {
    // Feed across two chunks: terminator in first, second finishes the line.
    let out = feed_chunks(&["Hello ", "world."]);
    assert_eq!(out, vec!["Hello world."]);
}

#[test]
fn test_multiple_terminators_in_one_chunk() {
    let out = feed_chunks(&["a.b!c?d"]);
    assert_eq!(out, vec!["a.", "b!", "c?", "d"]);
}

#[test]
fn test_empty_input_produces_no_output() {
    let out = feed_chunks(&[""]);
    assert!(out.is_empty());
}

// ── Code block detection via triple backticks ───────────────────────────────

#[test]
fn test_triple_backticks_open_code_block() {
    let out = feed_chunks(&["Some text.\n```\ncode\n"]);
    // "Some text." emits on '.', newline is consumed (code mode).
    // "code\n" emits on newline (code mode).
    assert!(out.contains(&"Some text.".to_string()));
    assert!(out.contains(&"code".to_string()));
}

#[test]
fn test_triple_backticks_close_code_block() {
    let out = feed_chunks(&["```\ncode\n```\nAfter text."]);
    // Inside code block: "code" emitted on newline.
    // "```" closes code block.
    // "After text." emitted on '.'.
    assert!(out.contains(&"code".to_string()));
    assert!(out.contains(&"After text.".to_string()));
}

#[test]
fn test_open_close_code_block_full_cycle() {
    let out = feed_chunks(&["Before.\n```\nfn foo() {}\n```\nAfter."]);
    // Fence lines (```) are trimmed but emitted; code lines not split on '.'.
    assert_eq!(out, vec!["Before.", "```", "fn foo() {}", "```", "After."]);
}

#[test]
fn test_incomplete_backtick_run_not_treated_as_fence() {
    // Only two backticks — should not toggle code block mode.
    let out = feed_chunks(&["text. ``nope. more."]);
    assert_eq!(out, vec!["text.", "``nope.", "more."]);
}

#[test]
fn test_backtick_run_split_across_chunks() {
    // "``" arrives in one chunk, "`" in the next — total ≥ 3 → toggle.
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    let out1 = r.handle_event(text_delta("``"));
    assert!(out1.text_messages.is_empty());
    // Backtick run is 2, no toggle yet — backtick pushed to buffer.
    let out2 = r.handle_event(text_delta("`code\n"));
    // Run reaches 3 on '`', toggles code on; 'c' resets run, toggles code off.
    // No code mode active → emit on '\n'.
    assert!(out2.text_messages.iter().any(|l| l.contains("code")));
    let out3 = r.handle_event(text_delta("```\nend."));
    assert!(out3.text_messages.iter().any(|l| l.contains("end.")));
}

// ── Code block periods are NOT split ────────────────────────────────────────

#[test]
fn test_code_block_periods_not_split() {
    let out = feed_chunks(&["```\nfoo.bar.baz\n```\n"]);
    // "foo.bar.baz" must remain intact — no split on '.'.
    assert!(out.contains(&"foo.bar.baz".to_string()));
}

#[test]
fn test_code_block_sentence_not_split() {
    let out = feed_chunks(&["```\nHello. World!\n```\n"]);
    // Both periods and exclamation marks ignored inside code block.
    assert!(out.contains(&"Hello. World!".to_string()));
}

#[test]
fn test_code_block_only_splits_on_newline() {
    let out = feed_chunks(&["```\na.b\nc.d\ne.f\n```\n"]);
    // Three lines, each split only on '\n'.
    assert!(out.contains(&"a.b".to_string()));
    assert!(out.contains(&"c.d".to_string()));
    assert!(out.contains(&"e.f".to_string()));
}

// ── Threshold force output ──────────────────────────────────────────────────

#[test]
fn test_threshold_force_output() {
    // LINE_THRESHOLD = 100. Feed 120 chars with no terminator.
    let long = "a".repeat(120);
    let out = feed_chunks(&[&long]);
    // Force-emitted at threshold.
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], long);
}

#[test]
fn test_threshold_with_partial_terminator() {
    // 95 chars + terminator → should emit on terminator (96 < 100).
    let mut text = "b".repeat(95);
    text.push('.');
    let out = feed_chunks(&[&text]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], text);
}

// ── Flush ───────────────────────────────────────────────────────────────────

#[test]
fn test_flush_outputs_residual() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    r.handle_event(text_delta("residual text"));
    let out = r.flush();
    assert_eq!(out.text_messages, vec!["residual text"]);
}

#[test]
fn test_flush_after_empty_produces_nothing() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    let out = r.flush();
    assert!(out.text_messages.is_empty());
}

#[test]
fn test_flush_after_complete_line_produces_nothing() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    r.handle_event(text_delta("done."));
    let out = r.flush();
    assert!(out.text_messages.is_empty());
}

// ── Block end flushes residual ──────────────────────────────────────────────

#[test]
fn test_block_end_flushes_residual() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    r.handle_event(text_delta("partial"));
    let out = r.handle_event(block_end(0, ContentBlockType::Text));
    assert_eq!(out.text_messages, vec!["partial"]);
}

#[test]
fn test_block_end_after_complete_line_no_residual() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    r.handle_event(text_delta("done."));
    let out = r.handle_event(block_end(0, ContentBlockType::Text));
    assert!(out.text_messages.is_empty());
}

// ── State reset on new block start ──────────────────────────────────────────

#[test]
fn test_block_start_resets_line_buffer() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    r.handle_event(text_delta("partial"));
    // Start a new text block — should reset line buffer.
    r.handle_event(block_start(1, ContentBlockType::Text));
    let out = r.handle_event(text_delta("new."));
    assert_eq!(out.text_messages, vec!["new."]);
}
