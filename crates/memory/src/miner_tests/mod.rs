//! Unit tests for the memory miner.
//!
//! Covers transcript cleaning, Miner 1 extraction, Miner 2 entity
//! assignment, dedup logic, SQLite write operations, and edge cases.

mod config_tests;
mod entity_tests;
mod session_tests;
mod similarity_filter_tests;
mod sqlite_tests;
mod transcript_tests;

use crate::miner::{MiningEntity, MiningEvent, MiningEventCategory};

// ── Shared helpers ───────────────────────────────────────────────────

pub(crate) fn make_event(title: &str, category: MiningEventCategory) -> MiningEvent {
    let has_lesson = category != MiningEventCategory::Decision;
    MiningEvent {
        title: title.to_string(),
        summary: format!("Summary of {title}"),
        body: format!("Body of {title}"),
        category,
        lesson: if has_lesson {
            Some(format!("Lesson from {title}"))
        } else {
            None
        },
    }
}

pub(crate) fn make_entity(name: &str, typ: &str) -> MiningEntity {
    MiningEntity {
        entity_type: typ.to_string(),
        name: name.to_string(),
        description: format!("Desc of {name}"),
    }
}
