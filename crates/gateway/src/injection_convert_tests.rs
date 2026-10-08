//! Unit tests for the gateway-side searcher tuple → session slot conversion.
//!
//! The memory-typed half of the conversion
//! (`InjectedMemorySummary` → tuple parts) lives in the composition root
//! and is covered there; these tests lock the tuple → `MemoryInjection`
//! half: known position tags map to matching positions, unknown tags
//! degrade to `AfterCurrent`, and content / event IDs carry through.

use std::collections::HashSet;

use closeclaw_session::llm_session::InjectionPosition;

use super::injection_convert::slot_parts_to_session_injection;

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
