//! Conversion between the searcher tuple protocol and session slot types.
//!
//! The composition root (daemon) converts memory-owned payloads
//! (`InjectedMemorySummary`) onto the tuple protocol used by the
//! [`SearcherRunner`](crate::SearcherRunner) seam:
//! `(content, position_tag, injected_event_ids)`. This module maps those
//! parts onto the session-owned slot type (`MemoryInjection` /
//! `InjectionPosition`), keeping the memory-typed half of the conversion
//! out of the gateway.

use std::collections::HashSet;

/// Build the session-slot injection from the tuple protocol parts.
pub fn slot_parts_to_session_injection(
    content: String,
    position: &str,
    event_ids: HashSet<i64>,
) -> closeclaw_session::llm_session::MemoryInjection {
    let position_mode = match position {
        "before_next" => closeclaw_session::llm_session::InjectionPosition::BeforeNext,
        _ => closeclaw_session::llm_session::InjectionPosition::AfterCurrent,
    };
    closeclaw_session::llm_session::MemoryInjection {
        content,
        position_mode,
        injected_event_ids: event_ids,
        task_id: None,
    }
}
