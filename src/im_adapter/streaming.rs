//! StreamingRenderer — incremental rendering for LLM `StreamEvent` streams.
//!
//! See `docs/design/im_adapter/streaming-render.md` for the architecture.
//!
//! This module provides:
//! - [`LineBuffer`] — splits incoming text on sentence terminators
//!   (`。！？.!?\n`) when outside fenced code blocks, and on `\n` when
//!   inside. Forces emission when the buffer exceeds a character threshold.
//! - [`DefaultStreamingRenderer`] — implements [`StreamingRenderer`]:
//!   feeds Text deltas through [`LineBuffer`], accumulates non-Text blocks,
//!   and routes DSL lines to a dedicated slot in [`StreamingOutput`].
//! - [`StreamingOutput`] — incremental output struct carrying completed
//!   text lines, non-Text [`ContentBlock`]s, and parsed DSL lines.

use crate::llm::types::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent};
use crate::processor_chain::dsl_parser::DslParser;

/// Default threshold (in characters) for forcing buffer emission.
const LINE_THRESHOLD: usize = 100;

/// Incremental output produced by [`StreamingRenderer`] for a batch of events.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StreamingOutput {
    /// Completed text lines emitted by the line buffer.
    pub text_messages: Vec<String>,
    /// Non-Text content blocks completed in this batch.
    pub render_blocks: Vec<ContentBlock>,
    /// DSL lines (e.g. `::button[...]`) extracted from text content.
    pub dsl_lines: Vec<String>,
}

/// Line buffer for incremental text rendering.
///
/// Splits incoming text on sentence terminators (`。！？.!?\n`) when outside
/// fenced code blocks, and on `\n` when inside. Forces emission when the
/// buffer exceeds the configured threshold.
pub struct LineBuffer {
    buffer: String,
    in_code_block: bool,
    threshold: usize,
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LineBuffer {
    /// Create a new line buffer with the default threshold (100 chars).
    pub fn new() -> Self {
        Self::with_threshold(LINE_THRESHOLD)
    }

    /// Create a new line buffer with a custom threshold.
    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            buffer: String::new(),
            in_code_block: false,
            threshold,
        }
    }

    /// Reset the buffer and code-block state.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.in_code_block = false;
    }

    /// Feed a text chunk; returns any lines completed by this chunk.
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        if chunk.is_empty() {
            return Vec::new();
        }
        let mut emitted: Vec<String> = Vec::new();
        let mut current_line = std::mem::take(&mut self.buffer);
        let mut in_code = self.in_code_block;
        let mut backtick_run: usize = count_trailing_backticks(&current_line);

        for ch in chunk.chars() {
            if ch == '`' {
                backtick_run += 1;
                current_line.push(ch);
                continue;
            }
            if backtick_run >= 3 {
                in_code = !in_code;
            }
            backtick_run = 0;
            current_line.push(ch);

            let emit = if in_code {
                ch == '\n'
            } else {
                is_sentence_terminator(ch) || ch == '\n'
            };
            if emit {
                emitted.push(std::mem::take(&mut current_line));
            }
        }

        self.in_code_block = in_code;
        self.buffer = current_line;

        if self.buffer.chars().count() >= self.threshold {
            self.force_emit(&mut emitted);
        }
        emitted
    }

    /// Flush the buffer; returns remaining content if any.
    pub fn flush(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            None
        } else {
            self.in_code_block = false;
            Some(std::mem::take(&mut self.buffer))
        }
    }

    fn force_emit(&mut self, emitted: &mut Vec<String>) {
        if let Some((byte_idx, _)) = self.buffer.char_indices().nth(self.threshold) {
            let line: String = self.buffer.drain(..byte_idx).collect();
            emitted.push(line);
        } else {
            emitted.push(std::mem::take(&mut self.buffer));
        }
    }
}

fn is_sentence_terminator(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '.' | '!' | '?')
}

fn count_trailing_backticks(s: &str) -> usize {
    s.chars().rev().take_while(|&c| c == '`').count()
}

fn is_dsl_line(line: &str) -> bool {
    let parser = DslParser;
    !parser.parse(line).instructions.is_empty()
}

fn route_line(line: &str, out: &mut StreamingOutput) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    if is_dsl_line(trimmed) {
        out.dsl_lines.push(trimmed.to_string());
    } else {
        out.text_messages.push(line.to_string());
    }
}

/// Per-block accumulator for non-Text blocks (Thinking / ToolUse).
#[derive(Debug, Default)]
struct BlockAccumulator {
    /// Concatenated text content (used for Thinking).
    text: String,
    /// Tool call id (for ToolUse).
    tool_id: Option<String>,
    /// Tool name (for ToolUse).
    tool_name: Option<String>,
    /// Tool input JSON (for ToolUse).
    tool_input: String,
}

impl BlockAccumulator {
    /// Convert the accumulated data into a [`ContentBlock`].
    fn into_block(self, block_type: ContentBlockType) -> ContentBlock {
        match block_type {
            ContentBlockType::Thinking => ContentBlock::Thinking(self.text),
            ContentBlockType::ToolUse => ContentBlock::ToolUse {
                id: self.tool_id.unwrap_or_default(),
                name: self.tool_name.unwrap_or_default(),
                input: self.tool_input,
            },
            ContentBlockType::Text => ContentBlock::Text(self.text),
        }
    }
}

/// Trait for incremental rendering of LLM [`StreamEvent`] streams.
///
/// Implementors must be `Send` so the renderer can be driven from async
/// tasks. Each [`handle_event`](Self::handle_event) call returns the
/// incremental output produced by that event; [`flush`](Self::flush) is
/// called at `MessageEnd` to drain any remaining buffered content.
pub trait StreamingRenderer: Send {
    /// Process a single [`StreamEvent`] and return incremental output.
    fn handle_event(&mut self, event: StreamEvent) -> StreamingOutput;

    /// Flush any remaining buffered content; called at MessageEnd.
    fn flush(&mut self) -> StreamingOutput;
}

/// Default streaming renderer: line-buffered text + per-block accumulation.
pub struct DefaultStreamingRenderer {
    line_buffer: LineBuffer,
    current_block_type: Option<ContentBlockType>,
    current_block_index: Option<usize>,
    current_acc: Option<BlockAccumulator>,
}

impl Default for DefaultStreamingRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultStreamingRenderer {
    /// Create a new renderer.
    pub fn new() -> Self {
        Self {
            line_buffer: LineBuffer::new(),
            current_block_type: None,
            current_block_index: None,
            current_acc: None,
        }
    }

    fn handle_text_delta(&mut self, text: &str, out: &mut StreamingOutput) {
        for line in self.line_buffer.feed(text) {
            route_line(&line, out);
        }
    }

    fn handle_thinking_delta(&mut self, thinking: &str) {
        if let Some(acc) = self.current_acc.as_mut() {
            acc.text.push_str(thinking);
        }
    }

    fn handle_tool_id(&mut self, id: String) {
        if let Some(acc) = self.current_acc.as_mut() {
            acc.tool_id = Some(id);
        }
    }

    fn handle_tool_name(&mut self, name: String) {
        if let Some(acc) = self.current_acc.as_mut() {
            acc.tool_name = Some(name);
        }
    }

    fn handle_tool_input(&mut self, input: &str) {
        if let Some(acc) = self.current_acc.as_mut() {
            acc.tool_input.push_str(input);
        }
    }

    /// Reset all renderer state fields to initial values.
    fn reset_state(&mut self) {
        self.line_buffer.reset();
        self.current_block_type = None;
        self.current_block_index = None;
        self.current_acc = None;
    }

    /// Flush remaining content from LineBuffer and any open accumulator,
    /// routing output to `out`.
    fn flush_remaining(&mut self, out: &mut StreamingOutput) {
        if let Some(remaining) = self.line_buffer.flush() {
            route_line(&remaining, out);
        }
        if let (Some(acc), Some(bt)) = (self.current_acc.take(), self.current_block_type.take()) {
            out.render_blocks.push(acc.into_block(bt));
            self.current_block_index = None;
        }
    }

    fn handle_block_start(&mut self, index: usize, block_type: ContentBlockType) {
        self.current_block_type = Some(block_type);
        self.current_block_index = Some(index);
        match block_type {
            ContentBlockType::Text => self.line_buffer.reset(),
            ContentBlockType::Thinking | ContentBlockType::ToolUse => {
                self.current_acc = Some(BlockAccumulator::default());
            }
        }
    }

    fn handle_block_end(
        &mut self,
        index: usize,
        block_type: ContentBlockType,
        out: &mut StreamingOutput,
    ) {
        match block_type {
            ContentBlockType::Text => {
                if let Some(remaining) = self.line_buffer.flush() {
                    route_line(&remaining, out);
                }
            }
            ContentBlockType::Thinking | ContentBlockType::ToolUse => {
                if let Some(acc) = self.current_acc.take() {
                    out.render_blocks.push(acc.into_block(block_type));
                }
            }
        }
        if self.current_block_index == Some(index) {
            self.current_block_type = None;
            self.current_block_index = None;
        }
    }
}

impl StreamingRenderer for DefaultStreamingRenderer {
    fn handle_event(&mut self, event: StreamEvent) -> StreamingOutput {
        let mut out = StreamingOutput::default();
        match event {
            StreamEvent::BlockStart { index, block_type } => {
                self.handle_block_start(index, block_type);
            }
            StreamEvent::BlockDelta { delta, .. } => match delta {
                ContentDelta::Text { text } => self.handle_text_delta(&text, &mut out),
                ContentDelta::Thinking { thinking } => self.handle_thinking_delta(&thinking),
                ContentDelta::ToolUseId { id } => self.handle_tool_id(id),
                ContentDelta::ToolUseName { name } => self.handle_tool_name(name),
                ContentDelta::ToolUseInputChunk { input } => self.handle_tool_input(&input),
            },
            StreamEvent::BlockEnd { index, block_type } => {
                self.handle_block_end(index, block_type, &mut out);
            }
            StreamEvent::MessageEnd { .. } => {
                self.flush_remaining(&mut out);
                self.reset_state();
            }
            StreamEvent::Error { .. } => {
                self.reset_state();
            }
        }
        out
    }

    fn flush(&mut self) -> StreamingOutput {
        let mut out = StreamingOutput::default();
        if let Some(remaining) = self.line_buffer.flush() {
            route_line(&remaining, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let out = buf.feed("Hello world. 你好世界！\nDone?");
        assert_eq!(out, vec!["Hello world.", " 你好世界！", "\n", "Done?"]);
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
        assert_eq!(out[0].chars().count(), 10);
        assert_eq!(out[0], "a]long str");
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
        assert!(out.dsl_lines.is_empty());
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
    fn renderer_routes_dsl_line_to_dsl_lines() {
        let mut r = DefaultStreamingRenderer::new();
        r.handle_event(block_start(0, ContentBlockType::Text));
        let dsl = "::button[label:Yes;action:vote;value:1]";
        let out = r.handle_event(text_delta(&format!("{}\n", dsl)));
        assert_eq!(out.dsl_lines, vec![dsl]);
        assert!(out.text_messages.is_empty());
    }

    #[test]
    fn renderer_force_emits_long_text_at_threshold() {
        let mut r = DefaultStreamingRenderer::new();
        // No terminator in 150-char string; first 100 chars must be emitted.
        let long_text = "a".repeat(150);
        r.handle_event(block_start(0, ContentBlockType::Text));
        let out = r.handle_event(text_delta(&long_text));
        assert_eq!(out.text_messages.len(), 1);
        assert_eq!(out.text_messages[0].chars().count(), 100);
    }

    #[test]
    fn renderer_block_end_thinking_emits_render_block() {
        let mut r = DefaultStreamingRenderer::new();
        r.handle_event(block_start(0, ContentBlockType::Thinking));
        r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "Let me think.".to_string(),
            },
        });
        let out = r.handle_event(block_end(0, ContentBlockType::Thinking));
        assert_eq!(
            out.render_blocks,
            vec![ContentBlock::Thinking("Let me think.".to_string())]
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

    // --- Step 1.2: MessageEnd / Error / state reset tests ---

    fn message_end() -> StreamEvent {
        StreamEvent::MessageEnd {
            usage: None,
            finish_reason: None,
        }
    }

    fn error_event(msg: &str) -> StreamEvent {
        StreamEvent::Error {
            message: msg.to_string(),
        }
    }

    #[test]
    fn test_message_end_flushes_remaining_buffer() {
        let mut r = DefaultStreamingRenderer::new();
        r.handle_event(block_start(0, ContentBlockType::Text));
        // Partial text with no terminator — stays in the line buffer.
        r.handle_event(text_delta("remaining text"));
        let out = r.handle_event(message_end());
        assert_eq!(out.text_messages, vec!["remaining text"]);
    }

    #[test]
    fn test_message_end_resets_state() {
        let mut r = DefaultStreamingRenderer::new();
        // Round 1: partial text left in buffer.
        r.handle_event(block_start(0, ContentBlockType::Text));
        r.handle_event(text_delta("first round partial"));
        let out1 = r.handle_event(message_end());
        assert_eq!(out1.text_messages, vec!["first round partial"]);
        // State should be fully reset — verify by starting round 2.
        r.handle_event(block_start(0, ContentBlockType::Text));
        let out2 = r.handle_event(text_delta("second round."));
        assert_eq!(out2.text_messages, vec!["second round."]);
        // No leftover from round 1.
        assert_eq!(out2.text_messages.len(), 1);
    }

    #[test]
    fn test_message_end_flushes_thinking_accumulator() {
        let mut r = DefaultStreamingRenderer::new();
        r.handle_event(block_start(0, ContentBlockType::Thinking));
        r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "let me think".to_string(),
            },
        });
        let out = r.handle_event(message_end());
        assert_eq!(
            out.render_blocks,
            vec![ContentBlock::Thinking("let me think".to_string())]
        );
    }

    #[test]
    fn test_error_resets_state() {
        let mut r = DefaultStreamingRenderer::new();
        r.handle_event(block_start(0, ContentBlockType::Text));
        r.handle_event(text_delta("partial"));
        // Error should not flush, just reset.
        let out = r.handle_event(error_event("stream failed"));
        assert!(out.text_messages.is_empty());
        assert!(out.render_blocks.is_empty());
        // State should be clean — start a new round.
        r.handle_event(block_start(0, ContentBlockType::Text));
        let out2 = r.handle_event(text_delta("after error."));
        assert_eq!(out2.text_messages, vec!["after error."]);
    }

    #[test]
    fn test_consecutive_rounds_isolation() {
        let mut r = DefaultStreamingRenderer::new();
        // Round 1: partial text (no terminator) stays in buffer until
        // BlockEnd, which flushes it.
        r.handle_event(block_start(0, ContentBlockType::Text));
        r.handle_event(text_delta("round 1 partial"));
        let be1 = r.handle_event(block_end(0, ContentBlockType::Text));
        assert_eq!(be1.text_messages, vec!["round 1 partial"]);
        let me1 = r.handle_event(message_end());
        assert!(me1.text_messages.is_empty());
        // Round 2: independent partial text.
        r.handle_event(block_start(0, ContentBlockType::Text));
        r.handle_event(text_delta("round 2 partial"));
        let be2 = r.handle_event(block_end(0, ContentBlockType::Text));
        assert_eq!(be2.text_messages, vec!["round 2 partial"]);
        let me2 = r.handle_event(message_end());
        assert!(me2.text_messages.is_empty());
    }
}
