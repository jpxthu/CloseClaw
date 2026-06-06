//! GLM streaming SSE parsing utilities.
//!
//! GLM SSE stream format:
//! ```text
//! data: {"id":"...","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"..."}}]}
//! data: {"id":"...","choices":[{"finish_reason":"length","index":0,"delta":{"role":"assistant","content":""}}],"usage":{...}}
//! data: [DONE]
//! ```
//! - Delta chunks contain `delta.reasoning_content` (or `delta.content`)
//! - The final chunk has `finish_reason` non-null plus `usage` at chunk level
//! - `[DONE]` is the termination marker
//!
//! The actual streaming transport (`send_streaming`) lives in [`crate::llm::glm`]
//! which produces [`RawSseChunk`][crate::llm::types::RawSseChunk] via
//! [`SseStream`][crate::llm::provider::SseStream]. This module provides
//! shared SSE line-parsing helpers.

/// Parse an SSE line: `data: {...}` or `data: [DONE]`.
///
/// Returns the data payload for JSON lines and `"[DONE]"` for the
/// termination marker. Returns `None` for non-data lines, blank lines,
/// or non-JSON data prefixes.
#[allow(dead_code)]
pub(crate) fn parse_sse_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.starts_with("data:") {
        return None;
    }
    let data = trimmed[5..].trim(); // strip "data:"
    if data == "[DONE]" {
        return Some("[DONE]");
    }
    if !data.starts_with('{') {
        return None;
    }
    Some(data)
}

#[cfg(test)]
mod tests;
