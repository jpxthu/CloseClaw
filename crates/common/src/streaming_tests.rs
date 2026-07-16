//! Unit tests for [`super::streaming`].
//!
//! Covers LineBuffer splitting/flushing, DefaultStreamingRenderer
//! incremental rendering, DSL line detection, block accumulation,
//! and state reset on flush/MessageEnd.

use super::streaming::*;
use crate::processor::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent};

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

// --- LineBuffer ---

#[test]
fn line_buffer_pure_text_splits_on_punctuation() {
    let mut buf = LineBuffer::new();
    // English and Chinese sentence terminators, plus newlines.
    let out = buf.feed("Hello world. 你好世界!\nDone?");
    assert_eq!(out, vec!["Hello world.", " 你好世界!", "\n", "Done?"]);
}

#[test]
fn line_buffer_keeps_incomplete_across_feeds() {
    let mut buf = LineBuffer::new();
    assert!(buf.feed("Hello ").is_empty());
    let out = buf.feed("world.");
    assert_eq!(out, vec!["Hello world."]);
    assert_eq!(buf.flush(), None);
}

#[test]
fn line_buffer_code_block_only_splits_on_newline() {
    let mut buf = LineBuffer::new();
    // Inside the fenced code block, "foo.bar" must NOT split on '.'.
    let out = buf.feed("```\nfoo.bar.baz\n```\n");
    assert_eq!(out, vec!["```\n", "foo.bar.baz\n", "```\n"]);
}

#[test]
fn line_buffer_force_emits_at_threshold() {
    let mut buf = LineBuffer::with_threshold(10);
    let out = buf.feed("a]long string without any terminator");
    assert_eq!(out.len(), 1);
    // force_emit outputs entire buffer, not truncated at threshold.
    assert_eq!(out[0], "a]long string without any terminator");
    assert!(buf.flush().is_none());
}

#[test]
fn line_buffer_flush_and_reset() {
    let mut buf = LineBuffer::new();
    buf.feed("partial");
    assert_eq!(buf.flush(), Some("partial".to_string()));
    assert_eq!(buf.flush(), None);
    buf.feed("more");
    buf.reset();
    assert_eq!(buf.flush(), None);
}

// --- DefaultStreamingRenderer ---

#[test]
fn renderer_pure_text_emits_complete_lines() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    let out = r.handle_event(text_delta("Hello world."));
    assert_eq!(out.text_messages, vec!["Hello world."]);
    assert!(out.render_blocks.is_empty());
}

#[test]
fn renderer_with_code_block_preserves_inner_periods() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    let out = r.handle_event(text_delta("```\nfoo.bar.baz\n```\n"));
    // "foo.bar.baz" must be one line, no split on '.'.
    assert_eq!(out.text_messages, vec!["```\n", "foo.bar.baz\n", "```\n"]);
}

#[test]
fn renderer_routes_dsl_line_to_text_messages() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    let dsl = "::button[label:Yes;action:vote;value:1]";
    let out = r.handle_event(text_delta(&format!("{}\n", dsl)));
    assert_eq!(out.text_messages, vec![format!("{}\n", dsl)]);
}

#[test]
fn renderer_force_emits_long_text_at_threshold() {
    let mut r = DefaultStreamingRenderer::new();
    // No terminator in 150-char string; all 150 chars must be emitted at once.
    let long_text = "a".repeat(150);
    r.handle_event(block_start(0, ContentBlockType::Text));
    let out = r.handle_event(text_delta(&long_text));
    assert_eq!(out.text_messages.len(), 1);
    assert_eq!(out.text_messages[0].chars().count(), 150);
}

#[test]
fn renderer_block_end_thinking_emits_render_block() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Thinking));
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: "Let me think.".to_string(),
            signature: None,
        },
    });
    let out = r.handle_event(block_end(0, ContentBlockType::Thinking));
    assert_eq!(
        out.render_blocks,
        vec![ContentBlock::Thinking {
            thinking: "Let me think.".to_string(),
            signature: None
        }]
    );
}

#[test]
fn renderer_block_end_tool_use_assembles_block() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::ToolUse));
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ToolUseId {
            id: "call_123".to_string(),
        },
    });
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ToolUseName {
            name: "search".to_string(),
        },
    });
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ToolUseInputChunk {
            input: r#"{"q":"rust"}"#.to_string(),
        },
    });
    let out = r.handle_event(block_end(0, ContentBlockType::ToolUse));
    assert_eq!(
        out.render_blocks,
        vec![ContentBlock::ToolUse {
            id: "call_123".to_string(),
            name: "search".to_string(),
            input: r#"{"q":"rust"}"#.to_string(),
        }]
    );
}

#[test]
fn renderer_text_block_end_and_flush_drain_buffer() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    // Feed partial (no terminator).
    r.handle_event(text_delta("partial"));
    let out = r.handle_event(block_end(0, ContentBlockType::Text));
    assert_eq!(out.text_messages, vec!["partial"]);
    // After BlockEnd, flush should have nothing to emit.
    let after = r.flush();
    assert!(after.text_messages.is_empty());
}

#[test]
fn renderer_flush_resets_block_state() {
    let mut r = DefaultStreamingRenderer::new();
    // Start a Thinking block and feed some data.
    r.handle_event(block_start(0, ContentBlockType::Thinking));
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: "partial thought".to_string(),
            signature: None,
        },
    });
    // Flush mid-block - should reset block state.
    let out = r.flush();
    assert!(out.render_blocks.is_empty());
    // Now start a new Thinking block (index 1). If old state leaked,
    // the new BlockStart might not create a fresh accumulator.
    r.handle_event(block_start(1, ContentBlockType::Thinking));
    r.handle_event(StreamEvent::BlockDelta {
        index: 1,
        delta: ContentDelta::Thinking {
            thinking: "new thought".to_string(),
            signature: None,
        },
    });
    let out = r.handle_event(block_end(1, ContentBlockType::Thinking));
    // The block must contain ONLY the new thought, not the old one.
    assert_eq!(
        out.render_blocks,
        vec![ContentBlock::Thinking {
            thinking: "new thought".to_string(),
            signature: None
        }]
    );
}

#[test]
fn renderer_message_end_then_new_block() {
    let mut r = DefaultStreamingRenderer::new();
    // Simulate a normal text exchange, then MessageEnd.
    r.handle_event(block_start(0, ContentBlockType::Text));
    r.handle_event(text_delta("Hello."));
    r.handle_event(block_end(0, ContentBlockType::Text));
    // flush (called at MessageEnd).
    let out = r.flush();
    assert!(out.text_messages.is_empty());
    // After MessageEnd, start a fresh text block.
    r.handle_event(block_start(0, ContentBlockType::Text));
    let out = r.handle_event(text_delta("Fresh start."));
    assert_eq!(out.text_messages, vec!["Fresh start."]);
    // No render blocks leaked from the previous message.
    assert!(out.render_blocks.is_empty());
}

// ── ImageRef / AudioRef / FileRef name vs url ──────────────────────────────

#[test]
fn test_image_ref_name_and_url_independent() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Image));
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ImageRef {
            name: "photo.jpg".to_string(),
            url: "https://cdn.example.com/photo.jpg".to_string(),
        },
    });
    let out = r.handle_event(block_end(0, ContentBlockType::Image));
    assert_eq!(out.render_blocks.len(), 1);
    match &out.render_blocks[0] {
        ContentBlock::Image { name, url } => {
            assert_eq!(name, "photo.jpg");
            assert_eq!(url, "https://cdn.example.com/photo.jpg");
        }
        other => panic!("expected Image block, got {:?}", other),
    }
}

#[test]
fn test_audio_ref_name_and_url_independent() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Audio));
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::AudioRef {
            name: "recording.wav".to_string(),
            url: "https://cdn.example.com/recording.wav".to_string(),
        },
    });
    let out = r.handle_event(block_end(0, ContentBlockType::Audio));
    assert_eq!(out.render_blocks.len(), 1);
    match &out.render_blocks[0] {
        ContentBlock::Audio { name, url } => {
            assert_eq!(name, "recording.wav");
            assert_eq!(url, "https://cdn.example.com/recording.wav");
        }
        other => panic!("expected Audio block, got {:?}", other),
    }
}

#[test]
fn test_file_ref_name_and_url_independent() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::File));
    r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::FileRef {
            name: "report.pdf".to_string(),
            url: "https://cdn.example.com/report.pdf".to_string(),
        },
    });
    let out = r.handle_event(block_end(0, ContentBlockType::File));
    assert_eq!(out.render_blocks.len(), 1);
    match &out.render_blocks[0] {
        ContentBlock::File { name, url } => {
            assert_eq!(name, "report.pdf");
            assert_eq!(url, "https://cdn.example.com/report.pdf");
        }
        other => panic!("expected File block, got {:?}", other),
    }
}

// ── Mutex<DefaultStreamingRenderer> integration ────────────────────────────

#[test]
fn mutex_renderer_delegates_handle_event() {
    let mut renderer = std::sync::Mutex::new(DefaultStreamingRenderer::new());
    renderer.handle_event(block_start(0, ContentBlockType::Text));
    let out = renderer.handle_event(text_delta("Hello world."));
    assert_eq!(out.text_messages, vec!["Hello world."]);
    assert!(out.render_blocks.is_empty());
}

#[test]
fn mutex_renderer_delegates_flush() {
    let mut renderer = std::sync::Mutex::new(DefaultStreamingRenderer::new());
    renderer.handle_event(block_start(0, ContentBlockType::Text));
    renderer.handle_event(text_delta("partial"));
    let out = renderer.flush();
    assert_eq!(out.text_messages, vec!["partial"]);
}

// ── LineBuffer timeout tests ──────────────────────────────────────────────

use std::thread;
use std::time::Duration;

#[test]
fn test_check_timeout_before_timeout_returns_none() {
    let mut buf = LineBuffer::new();
    buf.feed("partial");
    // Immediately after feed, timeout hasn't elapsed.
    assert!(buf.check_timeout().is_none());
}

#[test]
fn test_check_timeout_after_timeout_returns_lines() {
    let mut buf = LineBuffer::new();
    buf.feed("partial data");
    thread::sleep(Duration::from_millis(250));
    let result = buf.check_timeout();
    assert!(result.is_some());
    let lines = result.unwrap();
    assert_eq!(lines, vec!["partial data"]);
    // Buffer should be drained after timeout triggers.
    assert!(buf.flush().is_none());
}

#[test]
fn test_feed_resets_timeout_timer() {
    let mut buf = LineBuffer::new();
    buf.feed("first");
    thread::sleep(Duration::from_millis(100));
    buf.feed("second"); // This resets the timer.
                        // Total ~250ms from first feed, but only ~150ms from second.
                        // Timer was reset by second feed, so check_timeout returns None.
    thread::sleep(Duration::from_millis(150));
    assert!(buf.check_timeout().is_none());
    // Now wait for the timeout to actually elapse from the second feed.
    thread::sleep(Duration::from_millis(200));
    let result = buf.check_timeout();
    assert!(result.is_some());
    assert_eq!(result.unwrap(), vec!["firstsecond"]);
}

#[test]
fn test_check_timeout_empty_buffer_returns_none() {
    let mut buf = LineBuffer::new();
    // No feed at all, buffer is empty.
    assert!(buf.check_timeout().is_none());
}

#[test]
fn test_check_timeout_disabled_returns_none() {
    let mut buf = LineBuffer::new().with_timeout(None);
    buf.feed("partial");
    thread::sleep(Duration::from_millis(250));
    // Timeout is disabled, so check_timeout should always return None.
    assert!(buf.check_timeout().is_none());
    // Data is still in the buffer though.
    assert_eq!(buf.flush(), Some("partial".to_string()));
}

// ── WholeBlock code block mode tests ──────────────────────────────────────

#[test]
fn test_whole_block_code_block_emits_at_close() {
    let mut buf = LineBuffer::new().with_code_block_mode(CodeBlockMode::WholeBlock);
    let mut all = Vec::new();
    all.extend(buf.feed("```rust\nline 1\nline 2\n```\n"));
    // WholeBlock mode: code block content is held until the closing fence.
    // The closing ```\n should emit the accumulated block (without the trailing ```).
    assert!(!all.is_empty());
    // Should have the opening fence and the accumulated code block.
    let joined = all.join("");
    assert!(joined.contains("line 1"));
    assert!(joined.contains("line 2"));
}

#[test]
fn test_whole_block_outside_codeblock_linebyline() {
    let mut buf = LineBuffer::new().with_code_block_mode(CodeBlockMode::WholeBlock);
    let mut all = Vec::new();
    // Outside code block, sentence terminators should trigger line-by-line output.
    all.extend(buf.feed("Hello world. "));
    assert!(
        all.iter().any(|l| l.contains("Hello world.")),
        "Expected sentence terminator to trigger line output outside code block: {:?}",
        all
    );
}

#[test]
fn test_line_by_line_codeblock_default_behavior() {
    let mut buf = LineBuffer::new(); // Default is LineByLine.
    let out1 = buf.feed("```\nfoo\n");
    assert_eq!(out1, vec!["```\n", "foo\n"]);
    let out2 = buf.feed("bar\n```\n");
    assert_eq!(out2, vec!["bar\n", "```\n"]);
}

// ── DefaultStreamingRenderer timeout tests ─────────────────────────────────

#[test]
fn test_renderer_check_timeout_delegates_to_line_buffer() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Text));
    r.handle_event(text_delta("buffered text"));
    thread::sleep(Duration::from_millis(250));
    let out = r.check_timeout();
    assert!(
        !out.text_messages.is_empty(),
        "Expected text_messages from timeout, got: {:?}",
        out
    );
    assert!(out
        .text_messages
        .iter()
        .any(|m| m.contains("buffered text")));
}

#[test]
fn test_renderer_with_code_block_mode_whole() {
    let mut r = DefaultStreamingRenderer::new().with_code_block_mode(CodeBlockMode::WholeBlock);
    r.handle_event(block_start(0, ContentBlockType::Text));
    // Feed code block in one chunk.
    let out = r.handle_event(text_delta("```rust\nfn main() {}\n```\n"));
    // In WholeBlock mode, the code block should be emitted as one piece
    // when the closing fence arrives.
    let joined = out.text_messages.join("");
    assert!(
        joined.contains("fn main() {}"),
        "Expected code block content in output: {:?}",
        out.text_messages
    );
}

// ── Edge cases ────────────────────────────────────────────────────────────

#[test]
fn test_check_timeout_just_under_threshold() {
    let mut buf = LineBuffer::new();
    buf.feed("partial");
    thread::sleep(Duration::from_millis(100));
    // 100ms < 200ms timeout, should return None.
    assert!(buf.check_timeout().is_none());
}

#[test]
fn test_line_buffer_with_timeout_none_never_emits() {
    let mut buf = LineBuffer::new().with_timeout(None);
    buf.feed("data");
    thread::sleep(Duration::from_millis(250));
    assert!(buf.check_timeout().is_none());
    thread::sleep(Duration::from_millis(500));
    assert!(buf.check_timeout().is_none());
}
