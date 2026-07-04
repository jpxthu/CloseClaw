//! StreamingRenderer — incremental rendering for LLM `StreamEvent` streams.
//!
//! See `docs/design/im_adapter/streaming-render.md` for the architecture.
//!
//! This module provides:
//! - [`LineBuffer`] — splits incoming text on sentence terminators
//!   (`。！？.!?\n`) when outside fenced code blocks, and on `\n` when
//!   inside. Forces emission when the buffer exceeds a character threshold.
//! - [`DefaultStreamingRenderer`] — implements [`StreamingRenderer`]:
//!   feeds Text deltas through [`LineBuffer`] and accumulates non-Text blocks.
//! - [`StreamingOutput`] — incremental output struct carrying completed
//!   text lines and non-Text [`ContentBlock`]s.

use closeclaw_common::processor::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent};

/// Default threshold (in characters) for forcing buffer emission.
const LINE_THRESHOLD: usize = 100;

pub use closeclaw_common::im_plugin::StreamingOutput;

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
        if !self.buffer.is_empty() {
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

fn route_line(line: &str, out: &mut StreamingOutput) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    out.text_messages.push(line.to_string());
}

/// Per-block accumulator for non-Text blocks (Thinking / ToolUse).
#[derive(Debug, Default)]
struct BlockAccumulator {
    /// Concatenated text content (used for Thinking).
    text: String,
    /// Resource identifier for media reference blocks (Image/Audio/File).
    name: Option<String>,
    /// URL for media reference blocks (Image/Audio/File).
    url: Option<String>,
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
            ContentBlockType::Thinking => ContentBlock::Thinking {
                thinking: self.text,
                signature: None,
            },
            ContentBlockType::ToolUse => ContentBlock::ToolUse {
                id: self.tool_id.unwrap_or_default(),
                name: self.tool_name.unwrap_or_default(),
                input: self.tool_input,
            },
            ContentBlockType::ToolResult => ContentBlock::ToolResult {
                tool_call_id: self.tool_id.unwrap_or_default(),
                content: self.text,
            },
            ContentBlockType::Image => ContentBlock::Image {
                name: self.name.unwrap_or_default(),
                url: self.url.unwrap_or_default(),
            },
            ContentBlockType::Audio => ContentBlock::Audio {
                name: self.name.unwrap_or_default(),
                url: self.url.unwrap_or_default(),
            },
            ContentBlockType::File => ContentBlock::File {
                name: self.name.unwrap_or_default(),
                url: self.url.unwrap_or_default(),
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

    fn handle_block_start(&mut self, index: usize, block_type: ContentBlockType) {
        self.current_block_type = Some(block_type);
        self.current_block_index = Some(index);
        match block_type {
            ContentBlockType::Text => self.line_buffer.reset(),
            ContentBlockType::Thinking
            | ContentBlockType::ToolUse
            | ContentBlockType::ToolResult
            | ContentBlockType::Image
            | ContentBlockType::Audio
            | ContentBlockType::File => {
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
            ContentBlockType::Thinking
            | ContentBlockType::ToolUse
            | ContentBlockType::ToolResult
            | ContentBlockType::Image
            | ContentBlockType::Audio
            | ContentBlockType::File => {
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
                ContentDelta::Thinking { thinking, .. } => self.handle_thinking_delta(&thinking),
                ContentDelta::ToolUseId { id } => self.handle_tool_id(id),
                ContentDelta::ToolUseName { name } => self.handle_tool_name(name),
                ContentDelta::ToolUseInputChunk { input } => self.handle_tool_input(&input),
                ContentDelta::ToolResultText { text } => {
                    self.handle_thinking_delta(&text);
                }
                ContentDelta::ImageRef { name, url } => {
                    if let Some(acc) = self.current_acc.as_mut() {
                        acc.name = Some(name);
                        acc.url = Some(url);
                    }
                }
                ContentDelta::AudioRef { name, url } => {
                    if let Some(acc) = self.current_acc.as_mut() {
                        acc.name = Some(name);
                        acc.url = Some(url);
                    }
                }
                ContentDelta::FileRef { name, url } => {
                    if let Some(acc) = self.current_acc.as_mut() {
                        acc.name = Some(name);
                        acc.url = Some(url);
                    }
                }
            },
            StreamEvent::BlockEnd { index, block_type } => {
                self.handle_block_end(index, block_type, &mut out);
            }
            StreamEvent::MessageEnd { .. } | StreamEvent::Error { .. } => {}
        }
        out
    }

    fn flush(&mut self) -> StreamingOutput {
        let mut out = StreamingOutput::default();
        if let Some(remaining) = self.line_buffer.flush() {
            route_line(&remaining, &mut out);
        }
        self.current_block_type = None;
        self.current_block_index = None;
        self.current_acc = None;
        out
    }
}
