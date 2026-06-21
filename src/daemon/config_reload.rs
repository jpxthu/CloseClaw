//! Config Hot Reload Initialization
//!
//! Thin initialization entry point for config hot-reload at daemon startup.
//! Delegates file watching and event dispatch to [`ConfigReloadManager`].

use crate::agent::registry::AgentRegistry;
use crate::config::events::ConfigChangeEvent;
use crate::config::manager::ConfigManager;
use crate::config::reload::{ConfigReloadManager, WatcherHandle};
use crate::gateway::SessionManager;
use std::sync::Arc;
use tracing::info;

/// RAII handle for the config hot-reload system.
///
/// Dropping this stops the underlying filesystem watcher. The config-change
/// subscriber task runs for the lifetime of the tokio runtime.
pub(crate) struct ConfigWatcherHandle {
    _watcher: WatcherHandle,
}

/// Spawn a background task that subscribes to config change events and
/// notifies the [`SessionManager`].
///
/// When a [`ConfigChangeEvent::Reloaded`] arrives, the subscriber calls
/// [`SessionManager::notify_config_changed`]. `Failed` events are logged
/// but do not trigger a session notification.
fn spawn_config_change_subscriber(
    config_manager: Arc<ConfigManager>,
    session_manager: Arc<SessionManager>,
) {
    let mut rx = config_manager.subscribe_config_changes();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ConfigChangeEvent::Reloaded { section }) => {
                    info!(
                        section = %section,
                        "config change event received, notifying sessions"
                    );
                    session_manager.notify_config_changed(section).await;
                }
                Ok(ConfigChangeEvent::Failed { section, error }) => {
                    tracing::warn!(
                        section = %section,
                        error = %error,
                        "config change event failed, skipping session notification"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "config change subscriber lagged, missed events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("config change broadcast channel closed, subscriber exiting");
                    break;
                }
            }
        }
    });
}

/// Initialize config hot-reload: create a [`ConfigReloadManager`], start the
/// file watcher, and subscribe to config change events.
///
/// Returns a [`ConfigWatcherHandle`] (RAII: stops watching on drop).
pub(crate) fn init_config_hot_reload(
    config_dir: &str,
    config_manager: Arc<ConfigManager>,
    agent_registry: Arc<AgentRegistry>,
    session_manager: Arc<SessionManager>,
) -> anyhow::Result<ConfigWatcherHandle> {
    let manager = ConfigReloadManager::with_defaults(
        Arc::clone(&config_manager),
        Arc::clone(&agent_registry),
    );

    let watcher = manager
        .watch(config_dir)
        .map_err(|e| anyhow::anyhow!("failed to start config hot-reload watcher: {}", e))?;

    spawn_config_change_subscriber(config_manager, session_manager);

    info!("config hot-reload initialized, delegating to ConfigReloadManager");

    Ok(ConfigWatcherHandle { _watcher: watcher })
}

#[cfg(test)]
#[path = "config_reload_tests.rs"]
mod tests;
