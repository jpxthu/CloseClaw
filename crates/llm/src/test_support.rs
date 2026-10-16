//! Crate-internal helpers shared by unit test modules (`#[cfg(test)]` only).

use crate::retry::CooldownManager;
use crate::types::RawSseChunk;
use std::sync::Arc;

/// Cooldown manager isolated from the real home dir (TempDir-backed persist path).
///
/// The returned `TempDir` must be kept alive for as long as the manager is used.
pub(crate) fn isolated_cooldown() -> (tempfile::TempDir, Arc<CooldownManager>) {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let manager = CooldownManager::with_path(dir.path().join("llm_cooldowns.json"));
    (dir, Arc::new(manager))
}

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
