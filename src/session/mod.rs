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
pub mod events;
pub mod persistence;
pub mod recovery;
pub mod storage;

// Re-export commonly used types
pub use bootstrap::{BootstrapContext, BootstrapProtection, BootstrapRegion};
pub use events::{CheckpointTrigger, ModeSwitchEvent, UserIntent};
pub use persistence::{
    CheckpointManager, PersistenceError, PersistenceService, ReasoningMode, SessionCheckpoint,
};
