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
// Single definition of the owner-notification target parser (issue #3241):
// config_watcher's Failed-event path reuses this implementation.
pub(crate) use reload::parse_owner_target;

#[cfg(test)]
mod manager_reload_tests;
#[cfg(test)]
mod reload_tests;
