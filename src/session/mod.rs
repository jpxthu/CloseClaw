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

pub use closeclaw_session::bootstrap;
pub use closeclaw_session::checkpoint_manager;
pub mod compaction;
#[cfg(test)]
pub mod compaction_async_tests;
pub use closeclaw_session::events;
pub mod llm_caller;
#[cfg(test)]
pub mod pending_operations_tests;
pub use closeclaw_session::persistence;
pub use closeclaw_session::recovery;
pub use closeclaw_session::storage;
pub mod sweeper;
#[cfg(test)]
pub mod sweeper_tests;
pub use closeclaw_session::workspace;

// Re-export commonly used types
pub use bootstrap::{BootstrapContext, BootstrapProtection, BootstrapRegion};
pub use checkpoint_manager::CheckpointManager;
pub use compaction::{CompactConfig, CompactionResult, CompactionService, TokenWarningState};
pub use events::{CheckpointTrigger, ModeSwitchEvent, UserIntent};
pub use persistence::{
    PendingMessage, PersistenceError, PersistenceService, ReasoningLevel, ReasoningMode,
    SessionCheckpoint,
};
pub use recovery::SpawnTree;
