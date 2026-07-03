//! closeclaw-session: Session management crate.
//!
//! This crate contains the core session modules extracted from the main
//! closeclaw crate. It provides persistence, checkpoint management,
//! bootstrap, storage, and recovery functionality for agent sessions.

pub mod active_searcher;
pub mod bootstrap;
pub mod checkpoint_manager;
pub mod compaction;
pub mod events;
pub mod persistence;
pub mod recovery;
pub mod storage;
pub mod workspace;

#[cfg(test)]
mod active_searcher_tests;
#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod persistence_tests;
