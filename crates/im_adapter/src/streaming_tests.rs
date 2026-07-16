//! Unit tests for [`super::streaming`].
//!
//! Covers LineBuffer splitting/flushing, DefaultStreamingRenderer
//! incremental rendering, DSL line detection, block accumulation,
//! and state reset on flush/MessageEnd.

use super::streaming::*;
use closeclaw_common::processor::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent};

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

// ── Gap 3: Image/Audio/File blocks skip streaming rendering ──────────────

/// Gap 3: Image blocks do NOT produce render_blocks.
#[test]
fn test_image_ref_no_render_block() {
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
    assert!(
        out.render_blocks.is_empty(),
        "Image blocks should not produce render_blocks"
    );
}

/// Gap 3: Audio blocks do NOT produce render_blocks.
#[test]
fn test_audio_ref_no_render_block() {
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
    assert!(
        out.render_blocks.is_empty(),
        "Audio blocks should not produce render_blocks"
    );
}

/// Gap 3: File blocks do NOT produce render_blocks.
#[test]
fn test_file_ref_no_render_block() {
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
    assert!(
        out.render_blocks.is_empty(),
        "File blocks should not produce render_blocks"
    );
}

/// Gap 3: Image/Audio/File deltas are silently ignored by the renderer.
#[test]
fn test_image_audio_file_deltas_ignored() {
    let mut r = DefaultStreamingRenderer::new();
    r.handle_event(block_start(0, ContentBlockType::Image));
    let out1 = r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ImageRef {
            name: "a.jpg".to_string(),
            url: "https://x.com/a.jpg".to_string(),
        },
    });
    let out2 = r.handle_event(StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::ImageRef {
            name: "b.jpg".to_string(),
            url: "https://x.com/b.jpg".to_string(),
        },
    });
    assert!(out1.text_messages.is_empty());
    assert!(out1.render_blocks.is_empty());
    assert!(out2.text_messages.is_empty());
    assert!(out2.render_blocks.is_empty());
    let out3 = r.handle_event(block_end(0, ContentBlockType::Image));
    assert!(out3.render_blocks.is_empty());
}
