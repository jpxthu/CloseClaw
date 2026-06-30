//! Config hot-reload module (daemon-level).
//!
//! Contains the `ConfigReloadManager` that watches config files for changes
//! and orchestrates reload via `ConfigManager`. This module stays in the
//! main crate because it depends on `AgentRegistry` (daemon-level glue code).

pub mod reload;

// Re-export the main types for convenience
pub use reload::{ConfigReloadManager, WatcherHandle};

#[cfg(test)]
mod manager_reload_tests;
