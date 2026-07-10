//! Daemon-level reload callback implementation.
//!
//! Implements [`ReloadCallback`] for daemon-specific orchestration:
//! agent registry sync, permissions reload, and session provider rebuild.

use std::path::Path;

use closeclaw_agent::registry::AgentRegistry;
use closeclaw_config::manager::ConfigManager;
use closeclaw_config::ReloadCallback;
use tracing::info;
use tracing::warn;

/// Daemon-level reload callback.
///
/// Handles agent registry sync, permissions reload, and session
/// provider rebuild after config changes.
pub struct DaemonReloadCallback {
    agent_registry: std::sync::Arc<AgentRegistry>,
}

impl DaemonReloadCallback {
    /// Create a new daemon reload callback.
    pub fn new(agent_registry: std::sync::Arc<AgentRegistry>) -> Self {
        Self { agent_registry }
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
