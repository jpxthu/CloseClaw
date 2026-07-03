//! StreamingRenderer — trait for incremental rendering of LLM `StreamEvent` streams.
//!
//! Provides only the [`StreamingRenderer`] trait definition (interface contract).
//! Implementations live in `closeclaw-im-adapter`.

use crate::im_plugin::StreamingOutput;
use crate::processor::StreamEvent;

/// Trait for incremental streaming rendering of LLM events.
pub trait StreamingRenderer: Send {
    /// Process a single [`StreamEvent`] and return incremental output.
    fn handle_event(&mut self, event: StreamEvent) -> StreamingOutput;

    /// Flush any remaining buffered content; called at MessageEnd.
    fn flush(&mut self) -> StreamingOutput;
}
