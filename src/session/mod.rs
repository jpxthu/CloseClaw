//! Session Persistence Layer
//!
//! Provides checkpoint-based session persistence for gateway restart recovery.
//!
//! # Architecture
//!
//! - [`persistence`] — Core data structures and [`PersistenceService`] trait
//! - [`storage`] — Pluggable storage backends (memory, Redis, etc.)
//! - [`recovery`] — Session recovery service for gateway startup
//! - [`events`] — Checkpoint trigger event definitions
//! - [`bootstrap`] — Bootstrap context protection during compaction

pub mod compaction;
#[cfg(test)]
pub mod compaction_async_tests;
pub mod llm_caller;
#[cfg(test)]
pub mod pending_operations_tests;
pub mod sweeper;
#[cfg(test)]
pub mod sweeper_tests;

// Re-export session compaction types directly from the session crate
pub use closeclaw_session::compaction::{
    CompactConfig, CompactionResult, CompactionService, TokenWarningState,
};
