//! closeclaw-session: Session management crate.
//!
//! This crate contains the core session modules extracted from the main
//! closeclaw crate. It provides persistence, checkpoint management,
//! bootstrap, storage, and recovery functionality for agent sessions.

pub mod active_searcher;
pub mod background;
pub mod bootstrap;
pub mod checkpoint_manager;
pub mod compaction;
pub mod events;
pub mod llm_session;
pub mod persistence;
pub mod plan_archive;
pub mod plan_file;
pub mod recovery;
pub mod run_health;
pub mod storage;
pub mod workspace;

#[cfg(test)]
mod active_searcher_tests;
#[cfg(test)]
mod background_tests;
#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod plan_archive_tests;
#[cfg(test)]
mod plan_file_tests;
