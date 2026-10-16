//! Shared helper utilities for e2e tests.
//!
//! Centralizes daemon spawn logic, readiness polling, and lifecycle
//! assertions to avoid duplication across `sigterm_tests`,
//! `agent_profile_tests`, `shutdown_checkpoint_tests`, and
//! `gateway_restart_turn_tests`. Daemon spawning lives in [`daemon_spawn`],
//! readiness polling in [`daemon_ready`], and liveness/shutdown helpers in
//! [`daemon_lifecycle`] (whose `SHUTDOWN_TIMEOUT` and `sigterm_and_wait`
//! are item-gated on feature `fake-llm`). The minimal config-tree
//! scaffolding (empty `agents.json` + mandatory configs) lives in
//! [`config_tree`] and is **not** feature-gated, so cases built without
//! `fake-llm` can use it. The Chat-RPC client, fake-LLM server, and full
//! fake-LLM config-tree scaffolding live in the [`chat`], [`fake_llm`],
//! and [`config`] submodules (feature `fake-llm`, whose consumers are the
//! fake-LLM-backed test files).

pub mod config_tree;
#[cfg(test)]
mod config_tree_tests;

#[cfg(feature = "fake-llm")]
pub mod chat;
#[cfg(feature = "fake-llm")]
pub mod config;
#[cfg(feature = "fake-llm")]
pub mod daemon_guard;
pub mod daemon_lifecycle;
pub mod daemon_ready;
#[cfg(test)]
mod daemon_ready_tests;
pub mod daemon_spawn;
#[cfg(feature = "fake-llm")]
pub mod fake_llm;

// Re-exports keep the `helpers::<item>` paths stable for callers; in a
// binary crate rustc reports some as unused depending on the feature combo.
#[allow(unused_imports)]
pub use daemon_lifecycle::{assert_daemon_alive, assert_daemon_alive_with_context};
#[cfg(feature = "fake-llm")]
#[allow(unused_imports)]
pub use daemon_lifecycle::{sigterm_and_wait, SHUTDOWN_TIMEOUT};
#[allow(unused_imports)]
pub use daemon_ready::{wait_for_daemon_ready, wait_for_daemon_ready_with_timeout};
#[allow(unused_imports)]
pub use daemon_spawn::{closeclaw_binary, spawn_daemon};
