//! Conversion between memory-owned payload types and session slot types.
//!
//! `closeclaw-memory` owns its injection payload (`InjectedMemorySummary`)
//! and consumes `closeclaw_common::llm_types::InternalMessage` context
//! messages; the session slot contract (`MemoryInjection` /
//! `InjectionPosition`) is owned by `closeclaw-session`. This module is
//! the single mapping point in gateway so the `gateway → memory`
//! consumption can migrate wholesale when that edge is narrowed.

use std::collections::HashSet;

use closeclaw_common::llm_types::InternalMessage;
use closeclaw_common::processor::ContentBlock;
use closeclaw_memory::active_searcher::{InjectedMemorySummary, MemorySummaryPosition};
use closeclaw_session::active_searcher::SessionMessageSnapshot;

/// Convert session message snapshots into the internal messages consumed
/// by the memory crate's active-searcher prompt builder.
pub(crate) fn snapshots_to_internal_messages(
    snapshots: &[SessionMessageSnapshot],
) -> Vec<InternalMessage> {
    snapshots
        .iter()
        .map(|m| InternalMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            content_blocks: Some(vec![ContentBlock::Text(m.content.clone())]),
            tool_call_id: None,
        })
        .collect()
}

/// Map a memory-owned injection payload onto the tuple protocol used by
/// the session crate's `run_searcher` / `set_memory_injection` closures:
/// `(content, position_tag, injected_event_ids)`.
pub(crate) fn summary_to_slot_parts(
    summary: InjectedMemorySummary,
) -> (String, String, HashSet<i64>) {
    let position_tag = match summary.position {
        MemorySummaryPosition::BeforeNext => "before_next",
        MemorySummaryPosition::AfterCurrent => "after_current",
    };
    (
        summary.content,
        position_tag.to_string(),
        summary.injected_event_ids,
    )
}

/// Build the session-slot injection from the tuple protocol parts.
pub(crate) fn slot_parts_to_session_injection(
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
