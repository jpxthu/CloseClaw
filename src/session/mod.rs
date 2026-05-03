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

pub mod bootstrap;
pub mod checkpoint_manager;
pub mod compaction;
#[cfg(test)]
pub mod compaction_tests;
pub mod events;
pub mod persistence;
#[cfg(test)]
pub mod persistence_tests;
pub mod recovery;
pub mod storage;
pub mod sweeper;

// Re-export commonly used types
pub use bootstrap::{BootstrapContext, BootstrapProtection, BootstrapRegion};
pub use checkpoint_manager::CheckpointManager;
pub use compaction::{CompactConfig, CompactionResult, CompactionService, TokenWarningState};
pub use events::{CheckpointTrigger, ModeSwitchEvent, UserIntent};
pub use persistence::{PersistenceError, PersistenceService, ReasoningMode, SessionCheckpoint};
