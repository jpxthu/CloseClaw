//! Crate-internal helpers shared by test modules and the `fake` provider
//! (compiled under `#[cfg(any(test, feature = "fake-llm"))]`).

use crate::types::RawSseChunk;

/// Cooldown manager isolated from the real home dir (TempDir-backed persist path).
///
/// The returned `TempDir` must be kept alive for as long as the manager is used.
#[cfg(test)]
pub(crate) fn isolated_cooldown() -> (
    tempfile::TempDir,
    std::sync::Arc<crate::retry::CooldownManager>,
) {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let manager = crate::retry::CooldownManager::with_path(dir.path().join("llm_cooldowns.json"));
    (dir, std::sync::Arc::new(manager))
}

/// Creates a raw SSE chunk with the standard `"message"` event type.
#[cfg(test)]
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
