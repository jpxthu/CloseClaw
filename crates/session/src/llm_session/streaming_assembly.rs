//! Streaming content block assembly for the Session layer.
//!
//! [`StreamingContentAssembler`] accumulates [`ContentBlock`]s from
//! [`StreamEvent`]s as they flow through the stream, providing a
//! session-owned assembly point that aligns with the design doc's
//! requirement that the Session layer owns `ContentBlock[]` assembly.
//!
//! [`SessionStream`] wraps an inner event stream and accumulates
//! content blocks as events pass through, while still yielding each
//! event for downstream consumption (e.g., Gateway rendering).

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;

use closeclaw_common::processor::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedUsage,
};
use closeclaw_common::LLMError;

/// Accumulates [`ContentBlock`]s from [`StreamEvent`]s, owned by the
/// Session layer.
///
/// Processes each stream event and builds up a `Vec<ContentBlock>`:
/// - `BlockDelta(Text)` → appends text to the current Text block
/// - `BlockDelta(Thinking)` → appends thinking content
/// - `BlockDelta(ToolUse*)` → accumulates tool use fields
/// - `BlockEnd` → finalizes the current block
/// - `MessageEnd` → captures usage statistics
pub struct StreamingContentAssembler {
    content_blocks: Vec<ContentBlock>,
    /// Index → position in `content_blocks` for active block tracking.
    active_blocks: std::collections::HashMap<usize, usize>,
    /// Token usage captured from the `MessageEnd` event.
    usage: Option<UnifiedUsage>,
}

impl StreamingContentAssembler {
    /// Create a new empty assembler.
    pub fn new() -> Self {
        Self {
            content_blocks: Vec::new(),
            active_blocks: std::collections::HashMap::new(),
            usage: None,
        }
    }

    /// Process a single [`StreamEvent`], accumulating content blocks.
    pub fn process_event(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::BlockStart { index, block_type } => {
                let block = match block_type {
                    ContentBlockType::Text => ContentBlock::Text(String::new()),
                    ContentBlockType::Thinking => ContentBlock::Thinking {
                        thinking: String::new(),
                        signature: None,
                    },
                    ContentBlockType::ToolUse => ContentBlock::ToolUse {
                        id: String::new(),
                        name: String::new(),
                        input: String::new(),
                    },
                    ContentBlockType::ToolResult => ContentBlock::ToolResult {
                        tool_call_id: String::new(),
                        content: String::new(),
                    },
                    ContentBlockType::Image => ContentBlock::Image {
                        name: String::new(),
                        url: String::new(),
                    },
                    ContentBlockType::Audio => ContentBlock::Audio {
                        name: String::new(),
                        url: String::new(),
                    },
                    ContentBlockType::File => ContentBlock::File {
                        name: String::new(),
                        url: String::new(),
                    },
                };
                let pos = self.content_blocks.len();
                self.content_blocks.push(block);
                self.active_blocks.insert(*index, pos);
            }
            StreamEvent::BlockDelta { index, delta } => {
                if let Some(&pos) = self.active_blocks.get(index) {
                    self.apply_delta(pos, delta);
                }
            }
            StreamEvent::BlockEnd { index, .. } => {
                // Block is already fully assembled; just remove from active tracking.
                self.active_blocks.remove(index);
            }
            StreamEvent::MessageEnd { usage, .. } => {
                self.usage = usage.clone();
            }
            StreamEvent::Error { .. } => {
                // Errors don't produce content blocks; partial assembly is preserved.
            }
        }
    }

    /// Apply a [`ContentDelta`] to the block at the given position.
    fn apply_delta(&mut self, pos: usize, delta: &ContentDelta) {
        match delta {
            ContentDelta::Text { text } => {
                Self::apply_text_delta(&mut self.content_blocks[pos], text);
            }
            ContentDelta::Thinking {
                thinking,
                signature,
            } => {
                Self::apply_thinking_delta(&mut self.content_blocks[pos], thinking, signature);
            }
            ContentDelta::ToolUseId { id } => {
                Self::apply_tool_use_id(&mut self.content_blocks[pos], id);
            }
            ContentDelta::ToolUseName { name } => {
                Self::apply_tool_use_name(&mut self.content_blocks[pos], name);
            }
            ContentDelta::ToolUseInputChunk { input } => {
                Self::apply_tool_use_input(&mut self.content_blocks[pos], input);
            }
            ContentDelta::ToolResultText { text } => {
                Self::apply_tool_result_text(&mut self.content_blocks[pos], text);
            }
            ContentDelta::ImageRef { name, url } => {
                Self::apply_media_ref(&mut self.content_blocks[pos], name, url, "image");
            }
            ContentDelta::AudioRef { name, url } => {
                Self::apply_media_ref(&mut self.content_blocks[pos], name, url, "audio");
            }
            ContentDelta::FileRef { name, url } => {
                Self::apply_media_ref(&mut self.content_blocks[pos], name, url, "file");
            }
        }
    }

    fn apply_text_delta(block: &mut ContentBlock, text: &str) {
        if let ContentBlock::Text(ref mut t) = block {
            t.push_str(text);
        }
    }

    fn apply_thinking_delta(block: &mut ContentBlock, thinking: &str, signature: &Option<String>) {
        if let ContentBlock::Thinking {
            thinking: ref mut th,
            signature: ref mut sig,
        } = block
        {
            th.push_str(thinking);
            if sig.is_none() {
                *sig = signature.clone();
            }
        }
    }

    fn apply_tool_use_id(block: &mut ContentBlock, id: &str) {
        if let ContentBlock::ToolUse {
            id: ref mut tid, ..
        } = block
        {
            *tid = id.to_string();
        }
    }

    fn apply_tool_use_name(block: &mut ContentBlock, name: &str) {
        if let ContentBlock::ToolUse {
            name: ref mut n, ..
        } = block
        {
            *n = name.to_string();
        }
    }

    fn apply_tool_use_input(block: &mut ContentBlock, input: &str) {
        if let ContentBlock::ToolUse {
            input: ref mut i, ..
        } = block
        {
            i.push_str(input);
        }
    }

    fn apply_tool_result_text(block: &mut ContentBlock, text: &str) {
        if let ContentBlock::ToolResult {
            content: ref mut c, ..
        } = block
        {
            c.push_str(text);
        }
    }

    fn apply_media_ref(block: &mut ContentBlock, name: &str, url: &str, kind: &str) {
        let new_block = match kind {
            "audio" => ContentBlock::Audio {
                name: name.to_string(),
                url: url.to_string(),
            },
            "file" => ContentBlock::File {
                name: name.to_string(),
                url: url.to_string(),
            },
            _ => ContentBlock::Image {
                name: name.to_string(),
                url: url.to_string(),
            },
        };
        *block = new_block;
    }

    /// Consume the assembler and return the accumulated content blocks.
    pub fn into_content_blocks(self) -> Vec<ContentBlock> {
        self.content_blocks
    }

    /// Return a reference to the accumulated content blocks.
    pub fn content_blocks(&self) -> &[ContentBlock] {
        &self.content_blocks
    }

    /// Return the captured token usage from the `MessageEnd` event.
    pub fn usage(&self) -> Option<&UnifiedUsage> {
        self.usage.as_ref()
    }
}

impl Default for StreamingContentAssembler {
    fn default() -> Self {
        Self::new()
    }
}

/// Wraps an LLM stream, accumulating [`ContentBlock`]s as events pass through.
///
/// `SessionStream` implements [`Stream`] so it can be consumed by the
/// Gateway's outbound pipeline for real-time rendering. As each event
/// flows through, the internal [`StreamingContentAssembler`] builds
/// the final `ContentBlock[]`. After the stream is fully consumed,
/// call [`into_content_blocks`](Self::into_content_blocks) to extract
/// the assembled result.
pub struct SessionStream {
    inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
    assembler: StreamingContentAssembler,
    /// Whether the inner stream has been fully consumed.
    finished: bool,
}

impl SessionStream {
    /// Create a new `SessionStream` wrapping the given inner stream.
    pub fn new(inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>) -> Self {
        Self {
            inner,
            assembler: StreamingContentAssembler::new(),
            finished: false,
        }
    }

    /// Consume the stream wrapper and return the accumulated content blocks.
    ///
    /// **Must only be called after the stream has been fully consumed**
    /// (i.e., `poll_next` returned `Poll::Ready(None)`). Calling before
    /// the stream finishes yields a partial result.
    pub fn into_content_blocks(self) -> Vec<ContentBlock> {
        self.assembler.into_content_blocks()
    }

    /// Return a reference to the accumulated content blocks.
    pub fn content_blocks(&self) -> &[ContentBlock] {
        self.assembler.content_blocks()
    }

    /// Return the captured token usage from the `MessageEnd` event.
    pub fn usage(&self) -> Option<&UnifiedUsage> {
        self.assembler.usage()
    }

    /// Returns `true` if the inner stream has been fully consumed.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

impl Stream for SessionStream {
    type Item = Result<StreamEvent, LLMError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                this.assembler.process_event(&event);
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => {
                this.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

// SAFETY: SessionStream is Unpin because the inner Pin<Box<dyn Stream>>
// is itself Unpin (Box<T> is Unpin for all T). The assembler and
// finished flag are plain data with no pinning invariants.
impl Unpin for SessionStream {}
