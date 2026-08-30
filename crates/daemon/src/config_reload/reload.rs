//! Daemon-level reload callback implementation.
//!
//! Implements [`ReloadCallback`] for daemon-specific orchestration:
//! agent registry sync, permissions reload, and session provider rebuild.

use std::path::Path;

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_config::manager::ConfigManager;
use closeclaw_config::providers::SystemConfigData;
use closeclaw_config::ReloadCallback;
use closeclaw_gateway::Gateway;
use tokio::sync::mpsc;
use tracing::info;
use tracing::warn;

/// Daemon-level reload callback.
///
/// Handles agent registry sync, permissions reload, session
/// provider rebuild, and restart-class config detection.
pub struct DaemonReloadCallback {
    agent_registry: std::sync::Arc<AgentRegistry>,
    /// Channel to signal a restart-class config change to the Daemon.
    /// `String` is a human-readable change summary.
    daemon_restart_tx: Option<mpsc::Sender<String>>,
    /// Gateway reference for sending IM notifications on config
    /// validation failures. `None` in unit tests where Gateway
    /// construction is not practical.
    gateway: Option<std::sync::Arc<Gateway>>,
}

impl DaemonReloadCallback {
    /// Create a new daemon reload callback.
    pub fn new(
        agent_registry: std::sync::Arc<AgentRegistry>,
        gateway: std::sync::Arc<Gateway>,
    ) -> Self {
        Self {
            agent_registry,
            daemon_restart_tx: None,
            gateway: Some(gateway),
        }
    }

    /// Create a new daemon reload callback with restart signal support.
    pub fn with_restart_tx(
        agent_registry: std::sync::Arc<AgentRegistry>,
        daemon_restart_tx: mpsc::Sender<String>,
        gateway: std::sync::Arc<Gateway>,
    ) -> Self {
        Self {
            agent_registry,
            daemon_restart_tx: Some(daemon_restart_tx),
            gateway: Some(gateway),
        }
    }

    /// Create a daemon reload callback without a gateway reference.
    ///
    /// Used in unit tests where constructing a full Gateway is not
    /// practical.  `on_validation_failed` will log a warning but
    /// skip the IM notification.
    #[cfg(test)]
    pub fn new_for_test(agent_registry: std::sync::Arc<AgentRegistry>) -> Self {
        Self {
            agent_registry,
            daemon_restart_tx: None,
            gateway: None,
        }
    }

    #[cfg(test)]
    pub fn with_restart_tx_for_test(
        agent_registry: std::sync::Arc<AgentRegistry>,
        daemon_restart_tx: mpsc::Sender<String>,
    ) -> Self {
        Self {
            agent_registry,
            daemon_restart_tx: Some(daemon_restart_tx),
            gateway: None,
        }
    }

    /// Determine whether a config file change requires a gateway restart.
    ///
    /// Returns `true` for restart-class files:
    /// - `channels.json` — IM Adapter configuration
    /// - `gateway.json` — Gateway configuration
    /// - `models.json` — LLM Provider configuration
    pub(crate) fn is_restart_class(path: &Path) -> bool {
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(f) => f,
            None => return false,
        };
        matches!(filename, "channels.json" | "gateway.json" | "models.json")
    }

    /// Build a human-readable change summary for a restart-class config.
    fn describe_restart_class(path: &Path) -> String {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        match filename {
            "channels.json" => "IM Adapter config changed".to_string(),
            "gateway.json" => "Gateway config changed".to_string(),
            "models.json" => "LLM Provider config changed".to_string(),
            _ => format!("config changed: {}", filename),
        }
    }

    /// Reload agent configs and sync the `AgentRegistry`.
    ///
    /// Snapshots before reload; on failure, restores the previous
    /// in-memory state. Agent config files are backed up before
    /// reload but NOT rolled back on failure, per design doc.
    fn reload_agents_with_log(&self, path: &Path, config_manager: &ConfigManager) {
        info!(
            path = %path.display(),
            "agent config change detected, reloading agents"
        );

        let old_agents = config_manager.snapshot_agents();

        let agents_json = config_manager.config_dir().join("agents.json");
        let _ = config_manager.backup_manager().backup(&agents_json);

        let agents_dir = config_manager
            .config_dir()
            .parent()
            .unwrap_or(config_manager.config_dir())
            .join("agents");
        if agents_dir.exists() {
            for_each_agent_json(&agents_dir, |p| {
                let _ = config_manager.backup_manager().backup(p);
            });
        }

        if let Err(e) = config_manager.reload_agents() {
            warn!(error = %e, "failed to reload agent configs, restoring in-memory state");
            config_manager.restore_agents(old_agents);
            return;
        }

        let configs: Vec<_> = config_manager.agents().into_values().collect();
        self.agent_registry.reload(configs);
    }

    /// Reload permissions for a single agent.
    ///
    /// With lazy loading, `LazyAgentPermissions` detects mtime changes
    /// and reloads on next access. This method only logs for observability.
    fn reload_permissions_with_log(&self, path: &Path, _config_manager: &ConfigManager) {
        let Some(agent_id) = extract_agent_id_from_permissions_path(path) else {
            warn!(
                path = %path.display(),
                "cannot determine agent_id from permissions path, skipping reload"
            );
            return;
        };

        info!(
            agent_id = %agent_id,
            path = %path.display(),
            "permissions change detected — lazy loader will pick up changes on next access"
        );
    }
}

impl ReloadCallback for DaemonReloadCallback {
    fn on_agents_changed(&self, path: &Path, config_manager: &ConfigManager) {
        self.reload_agents_with_log(path, config_manager);
    }

    fn on_permissions_changed(&self, path: &Path, config_manager: &ConfigManager) {
        self.reload_permissions_with_log(path, config_manager);
    }

    fn on_session_reloaded(&self, config_manager: &ConfigManager) {
        config_manager.reload_session_provider();
    }

    fn on_config_file_changed(&self, path: &Path, _config_manager: &ConfigManager) {
        if !Self::is_restart_class(path) {
            return;
        }
        let summary = Self::describe_restart_class(path);
        info!(
            path = %path.display(),
            summary = %summary,
            "restart-class config change detected"
        );
        if let Some(ref tx) = self.daemon_restart_tx {
            if tx.try_send(summary).is_err() {
                warn!(
                    path = %path.display(),
                    "failed to send restart signal — channel full or closed"
                );
            }
        }
    }

    fn on_validation_failed(
        &self,
        section: closeclaw_config::ConfigSection,
        path: &Path,
        error: &str,
        config_manager: &ConfigManager,
    ) {
        warn!(
            section = %section,
            path = %path.display(),
            error = %error,
            "config validation failed, sending IM notification to owner"
        );
        let Some(ref gateway) = self.gateway else {
            warn!(
                section = %section,
                "no gateway available — skipping IM notification"
            );
            return;
        };
        if let Some((channel, chat_id)) = parse_owner_target(config_manager) {
            let msg = format!(
                "⚠️ Config reload failed for section `{}`: {}",
                section, error
            );
            let gateway = std::sync::Arc::clone(gateway);
            tokio::spawn(async move {
                if let Err(e) = gateway
                    .send_outbound_simplified(&chat_id, &channel, &msg)
                    .await
                {
                    warn!(
                        error = %e,
                        "failed to send config validation failure IM notification"
                    );
                }
            });
        } else {
            warn!(
                section = %section,
                "owner_display not configured — skipping IM notification"
            );
        }
    }
}

/// Parse the owner notification target from `SystemConfigData.commands.owner_display`.
///
/// Expects format `"platform:chat_id"`.
fn parse_owner_target(config_manager: &ConfigManager) -> Option<(String, String)> {
    let raw = config_manager
        .get_section_value(closeclaw_config::ConfigSection::System)
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

/// Extract agent_id from a permissions.json path.
pub(crate) fn extract_agent_id_from_permissions_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    if !parent.join("config.json").exists() {
        return None;
    }
    parent
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Iterate over `.json` files in the agents directory and apply `f` to each.
pub(crate) fn for_each_agent_json<F>(agents_dir: &Path, f: F)
where
    F: Fn(&Path),
{
    if let Ok(entries) = std::fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "json") {
                f(&path);
            }
        }
    }
}
