//! ArchiveSweeper test family: shared fixtures (`sweeper_test_utils`) plus
//! per-dimension cases (run_once, cascade, shutdown, purge, archive decision,
//! active-query gating).
//!
//! The whole family sits behind a single `#![cfg(test)]` gate below so the
//! crate root only needs a bare `mod sweeper_tests;` declaration.

#![cfg(test)]

mod sweeper_active_query_tests;
mod sweeper_archive_decision_tests;
mod sweeper_cascade_tests;
mod sweeper_purge_task_tests;
mod sweeper_run_once_tests;
mod sweeper_shutdown_tests;
mod sweeper_test_utils;
