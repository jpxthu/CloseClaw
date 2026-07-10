//! Config hot-reload module (daemon-level).
//!
//! Imports `ConfigReloadManager` and `WatcherHandle` from the config crate.
//! Contains the daemon-specific [`reload::DaemonReloadCallback`] that
//! handles agent registry sync and permissions reload.

pub mod reload;

// Re-export types from config crate for convenience
pub use closeclaw_config::{ConfigReloadManager, WatcherHandle};
// Re-export daemon-specific callback
pub use reload::DaemonReloadCallback;

#[cfg(test)]
mod manager_reload_tests;
