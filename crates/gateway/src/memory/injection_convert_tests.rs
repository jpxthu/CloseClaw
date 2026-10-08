//! Unit tests for the gateway-side memory injection conversion helpers.
//!
//! Covers the conversion behavior dimension required by the memory
//! dependency-alignment plan:
//! - Normal path: session message snapshots map onto
//!   `closeclaw_common::llm_types::InternalMessage` with field equivalence.
//! - Position mapping: memory-owned positions map onto the session slot
//!   tags, and the slot-tag → `MemoryInjection` direction falls back to
//!   `AfterCurrent` on unknown tags.
//! - Round trip: `InjectedMemorySummary` → slot parts → `MemoryInjection`
//!   preserves content, position semantics, and injected event IDs.

use std::collections::HashSet;

use closeclaw_common::llm_types::InternalMessage;
use closeclaw_common::processor::ContentBlock;
use closeclaw_memory::active_searcher::{InjectedMemorySummary, MemorySummaryPosition};
use closeclaw_session::active_searcher::SessionMessageSnapshot;
use closeclaw_session::llm_session::InjectionPosition;

use super::injection_convert::{
    slot_parts_to_session_injection, snapshots_to_internal_messages, summary_to_slot_parts,
};

fn snapshot(role: &str, content: &str) -> SessionMessageSnapshot {
    SessionMessageSnapshot {
        role: role.to_string(),
        content: content.to_string(),
    }
}

fn summary(
    content: &str,
    position: MemorySummaryPosition,
    event_ids: &[i64],
) -> InjectedMemorySummary {
    InjectedMemorySummary {
        content: content.to_string(),
        position,
        injected_event_ids: event_ids.iter().copied().collect(),
    }
}

// ── snapshots_to_internal_messages ──────────────────────────────────────

#[test]
fn test_snapshots_map_with_field_equivalence() {
    let snapshots = vec![
        snapshot("user", "hello there"),
        snapshot("assistant", "hi, how can I help?"),
    ];
    let messages = snapshots_to_internal_messages(&snapshots);
    assert_eq!(messages.len(), 2, "one internal message per snapshot");
    let expected: Vec<InternalMessage> = snapshots
        .iter()
        .map(|m| InternalMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            content_blocks: Some(vec![ContentBlock::Text(m.content.clone())]),
            tool_call_id: None,
        })
        .collect();
    assert_eq!(messages, expected, "role/content/blocks must map 1:1");
}

#[test]
fn test_empty_snapshot_list_maps_to_empty_messages() {
    let messages = snapshots_to_internal_messages(&[]);
    assert!(messages.is_empty(), "no snapshots must yield no messages");
}

// ── summary_to_slot_parts ───────────────────────────────────────────────

#[test]
fn test_before_next_position_maps_to_before_next_tag() {
    let (content, tag, ids) = summary_to_slot_parts(summary(
        "remembered fact",
        MemorySummaryPosition::BeforeNext,
        &[7, 9],
    ));
    assert_eq!(content, "remembered fact");
    assert_eq!(tag, "before_next");
    assert_eq!(ids, HashSet::from([7, 9]), "event ids pass through");
}

#[test]
fn test_after_current_position_maps_to_after_current_tag() {
    let (_, tag, ids) = summary_to_slot_parts(summary(
        "other fact",
        MemorySummaryPosition::AfterCurrent,
        &[3],
    ));
    assert_eq!(tag, "after_current");
    assert_eq!(ids, HashSet::from([3]));
}

// ── slot_parts_to_session_injection ─────────────────────────────────────

#[test]
fn test_known_tags_map_to_matching_positions() {
    let before_next =
        slot_parts_to_session_injection("c".to_string(), "before_next", HashSet::from([1]));
    assert_eq!(before_next.position_mode, InjectionPosition::BeforeNext);
    let after_current =
        slot_parts_to_session_injection("c".to_string(), "after_current", HashSet::from([1]));
    assert_eq!(after_current.position_mode, InjectionPosition::AfterCurrent);
}

#[test]
fn test_unknown_tag_falls_back_to_after_current() {
    let injection =
        slot_parts_to_session_injection("c".to_string(), "bogus-tag", HashSet::from([1]));
    assert_eq!(
        injection.position_mode,
        InjectionPosition::AfterCurrent,
        "unknown position tags must degrade to AfterCurrent"
    );
}

#[test]
fn test_session_injection_carries_content_ids_and_no_task() {
    let injection = slot_parts_to_session_injection(
        "carried content".to_string(),
        "before_next",
        HashSet::from([5, 6, 7]),
    );
    assert_eq!(injection.content, "carried content");
    assert_eq!(injection.injected_event_ids, HashSet::from([5, 6, 7]));
    assert!(
        injection.task_id.is_none(),
        "searcher injections have no task"
    );
}

// ── Round trip: summary → parts → session injection ─────────────────────

#[test]
fn test_summary_to_session_injection_round_trip_is_lossless() {
    for (position, tag) in [
        (MemorySummaryPosition::BeforeNext, "before_next"),
        (MemorySummaryPosition::AfterCurrent, "after_current"),
    ] {
        let payload = summary("round trip", position, &[11, 12]);
        let (content, position_tag, ids) = summary_to_slot_parts(payload);
        let injection = slot_parts_to_session_injection(content, &position_tag, ids);
        assert_eq!(injection.content, "round trip");
        let expected_mode = match position {
            MemorySummaryPosition::BeforeNext => InjectionPosition::BeforeNext,
            MemorySummaryPosition::AfterCurrent => InjectionPosition::AfterCurrent,
        };
        assert_eq!(
            injection.position_mode, expected_mode,
            "position semantics must survive the round trip via tag {tag}"
        );
        assert_eq!(
            injection.injected_event_ids,
            HashSet::from([11, 12]),
            "injected event ids must survive the round trip"
        );
    }
}
