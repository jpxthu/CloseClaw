//! ArchiveSweeper test family: shared fixtures (`sweeper_test_utils`) plus
//! per-dimension cases (run_once, cascade, shutdown, purge, archive decision,
//! active-query gating).
//!
//! The family is gated at its crate-root declaration (`#[cfg(test)]` on
//! `mod sweeper_tests;` in `lib.rs`), matching every other test module in
//! this crate; the submodules below carry no extra gate.

mod sweeper_active_query_tests;
mod sweeper_archive_decision_tests;
mod sweeper_cascade_tests;
mod sweeper_purge_task_tests;
mod sweeper_run_once_tests;
mod sweeper_shutdown_tests;
mod sweeper_test_utils;
