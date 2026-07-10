//! Config Hot Reload Initialization
//!
//! Thin initialization entry point for config hot-reload at daemon startup.
//! Delegates file watching and event dispatch to [`ConfigReloadManager`].

use crate::config_reload::reload::{ConfigReloadManager, WatcherHandle};
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_config::events::ConfigChangeEvent;
use closeclaw_config::manager::{ConfigManager, ConfigSection};
use closeclaw_config::providers::SystemConfigData;
use closeclaw_gateway::{Gateway, SessionManager};
use std::sync::Arc;
use tracing::{info, warn};

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
/// [`SessionManager::notify_config_changed`] with the section and the
/// latest config snapshot. `Failed` events trigger an IM notification
/// to the configured owner (if `owner_display` is set in `system.json`).
fn spawn_config_change_subscriber(
    config_manager: Arc<ConfigManager>,
    session_manager: Arc<SessionManager>,
    gateway: Arc<Gateway>,
) {
    let mut event_rx = config_manager.subscribe_config_changes();
    let mut snapshot_rx = config_manager.subscribe_config_snapshots();
    tokio::spawn(async move {
        loop {
            let event = event_rx.recv().await;
            match event {
                Ok(ConfigChangeEvent::Reloaded { section }) => {
                    info!(
                        section = %section,
                        "config change event received, notifying sessions"
                    );
                    // Block until the matching snapshot arrives.
                    let snapshot = match snapshot_rx.recv().await {
                        Ok(s) => s,
                        Err(e) => {
                            warn!(
                                section = %section,
                                error = %e,
                                "failed to receive config snapshot, skipping session notification"
                            );
                            continue;
                        }
                    };
                    session_manager
                        .notify_config_changed(section, snapshot)
                        .await;
                }
                Ok(ConfigChangeEvent::Failed { section, error }) => {
                    warn!(
                        section = %section,
                        error = %error,
                        "config change event failed, skipping session notification"
                    );
                    // Try to notify the owner via IM.
                    let target = parse_owner_target(&config_manager);
                    if let Some((channel, chat_id)) = target {
                        let msg = format!(
                            "⚠️ Config reload failed for section `{}`: {}",
                            section, error
                        );
                        if let Err(e) = gateway
                            .send_outbound_simplified(&chat_id, &channel, &msg)
                            .await
                        {
                            warn!(
                                error = %e,
                                "failed to send config reload failure notification to owner"
                            );
                        }
                    } else {
                        warn!(
                            section = %section,
                            "owner_display not configured — skipping IM notification"
                        );
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(missed = n, "config change subscriber lagged, missed events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("config change broadcast channel closed, subscriber exiting");
                    break;
                }
            }
        }
    });
}

/// Parse the owner notification target from `SystemConfigData.commands.owner_display`.
///
/// Expected format: `"platform:chat_id"` (e.g. `"feishu:oc_xxx"`).
/// Returns `None` if not configured or invalid.
fn parse_owner_target(config_manager: &ConfigManager) -> Option<(String, String)> {
    let raw = config_manager
        .get_section_value(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())?
        .commands?
        .owner_display?;
    let parts: Vec<&str> = raw.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        warn!(
            owner_display = %raw,
            "invalid owner_display format, expected 'platform:chat_id'"
        );
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
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
    gateway: Arc<Gateway>,
) -> anyhow::Result<ConfigWatcherHandle> {
    let mut manager = ConfigReloadManager::with_defaults(
        Arc::clone(&config_manager),
        Arc::clone(&agent_registry),
    );

    let watcher = manager
        .watch(config_dir)
        .map_err(|e| anyhow::anyhow!("failed to start config hot-reload watcher: {}", e))?;

    spawn_config_change_subscriber(config_manager, session_manager, gateway);

    info!("config hot-reload initialized, delegating to ConfigReloadManager");

    Ok(ConfigWatcherHandle { _watcher: watcher })
}

#[cfg(test)]
#[path = "config_reload_tests.rs"]
mod tests;
