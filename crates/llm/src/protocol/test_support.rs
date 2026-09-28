//! Shared test-support helpers for protocol tests.

use crate::types::RawSseChunk;

/// Creates a raw SSE chunk with the standard `"message"` event type.
pub(crate) fn make_sse_chunk(data: &str) -> RawSseChunk {
    make_sse_chunk_with_event("message", data)
}

/// Creates a raw SSE chunk with an explicit event type.
pub(crate) fn make_sse_chunk_with_event(event_type: &str, data: &str) -> RawSseChunk {
    RawSseChunk {
        event_type: event_type.to_string(),
        data: data.to_string(),
    }
}
