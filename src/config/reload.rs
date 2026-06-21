//! Config Hot Reload Manager
//!
//! Watches config files for changes and automatically reloads them via
//! `ConfigManager`. Handles debounce, file dispatch, and agent directory
//! changes (snapshot → reload → `AgentRegistry::sync`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use tracing::{debug, info, warn};

use super::manager::{ConfigManager, ConfigSection};
use crate::agent::registry::AgentRegistry;

/// Default debounce duration for file change events.
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// RAII handle that keeps the filesystem watcher alive.
///
/// Dropping this handle stops the underlying watcher.
#[derive(Debug)]
#[allow(dead_code)] // field is kept alive for RAII watcher lifecycle
pub struct WatcherHandle {
    watcher: RecommendedWatcher,
}

impl WatcherHandle {
    /// Explicitly stop watching (same as drop, but allows manual control).
    pub fn stop(self) {
        // Watcher stops when dropped
    }
}

/// Daemon-level config hot-reload manager.
///
/// Watches a set of config JSON files and the `agents/` directory.
/// On change, dispatches to `ConfigManager::reload_section()` or
/// `ConfigManager::reload_agents()` with debounce protection.
pub struct ConfigReloadManager {
    /// Shared config manager — provides load/reload/backup logic.
    config_manager: Arc<ConfigManager>,
    /// Shared agent registry — synced after agent config changes.
    agent_registry: Arc<AgentRegistry>,
    /// Debounce interval to avoid rapid reloads.
    debounce_duration: Duration,
}

impl ConfigReloadManager {
    /// Create a new `ConfigReloadManager`.
    ///
    /// The manager holds shared references to `ConfigManager` and
    /// `AgentRegistry`; it does not own them.
    pub fn new(
        config_manager: Arc<ConfigManager>,
        agent_registry: Arc<AgentRegistry>,
        debounce_duration: Duration,
    ) -> Self {
        Self {
            config_manager,
            agent_registry,
            debounce_duration,
        }
    }

    /// Create a new `ConfigReloadManager` with default debounce (500ms).
    pub fn with_defaults(
        config_manager: Arc<ConfigManager>,
        agent_registry: Arc<AgentRegistry>,
    ) -> Self {
        Self::new(config_manager, agent_registry, DEFAULT_DEBOUNCE)
    }

    /// Start watching config files under `config_dir`.
    ///
    /// Watches the following files (if they exist):
    /// - `models.json`, `channels.json`, `gateway.json`,
    ///   `plugins.json`, `system.json`
    /// - `agents.json` (treated as agent config, triggers `reload_agents`)
    /// - `agents/` directory (recursive, triggers `reload_agents`)
    ///
    /// Returns a `WatcherHandle` whose drop stops the watcher.
    pub fn watch(&self, config_dir: &str) -> Result<WatcherHandle, super::ConfigError> {
        let config_path = Path::new(config_dir);

        // Collect config file paths to watch
        let config_files: Vec<PathBuf> = [
            "models.json",
            "channels.json",
            "gateway.json",
            "plugins.json",
            "system.json",
        ]
        .iter()
        .map(|f| config_path.join(f))
        .collect();

        let agents_json_path = config_path.join("agents.json");
        let agents_dir = config_path.join("agents");

        // Create the mpsc channel for file change events
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

        // Build the recommended watcher
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.send(Ok(event));
                }
            },
            NotifyConfig::default(),
        )
        .map_err(|e| super::ConfigError::SchemaError(format!("Failed to create watcher: {}", e)))?;

        // Watch individual config files
        for path in &config_files {
            if path.exists() {
                watcher
                    .watch(path.as_ref(), RecursiveMode::NonRecursive)
                    .map_err(|e| {
                        super::ConfigError::SchemaError(format!(
                            "Failed to watch {:?}: {}",
                            path, e
                        ))
                    })?;
            }
        }

        // Watch agents.json (triggers reload_agents)
        if agents_json_path.exists() {
            watcher
                .watch(agents_json_path.as_ref(), RecursiveMode::NonRecursive)
                .map_err(|e| {
                    super::ConfigError::SchemaError(format!("Failed to watch agents.json: {}", e))
                })?;
        }

        // Watch agents/ directory recursively
        if agents_dir.exists() {
            watcher
                .watch(agents_dir.as_ref(), RecursiveMode::Recursive)
                .map_err(|e| {
                    super::ConfigError::SchemaError(format!("Failed to watch agents/: {}", e))
                })?;
        }

        // Spawn the debounce + dispatch loop
        let config_manager = Arc::clone(&self.config_manager);
        let agent_registry = Arc::clone(&self.agent_registry);
        let debounce = self.debounce_duration;

        std::thread::spawn(move || {
            run_reload_loop(rx, config_manager, agent_registry, debounce);
        });

        info!(config_dir = config_dir, "config hot-reload watcher started");

        Ok(WatcherHandle { watcher })
    }
}

// ---------------------------------------------------------------------------
// Internal event loop
// ---------------------------------------------------------------------------

/// Background loop: receive file change events, debounce, and dispatch.
fn run_reload_loop(
    rx: std::sync::mpsc::Receiver<notify::Result<Event>>,
    config_manager: Arc<ConfigManager>,
    agent_registry: Arc<AgentRegistry>,
    debounce: Duration,
) {
    let mut last_reload = std::time::Instant::now()
        .checked_sub(debounce * 2)
        .unwrap_or_else(std::time::Instant::now);

    for event_result in rx {
        let event = match event_result {
            Ok(e) => e,
            Err(e) => {
                warn!("config watcher event error: {}", e);
                continue;
            }
        };

        // Only react to create / modify / remove
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {}
            _ => continue,
        }

        // Debounce: skip if within the debounce window
        let now = std::time::Instant::now();
        if now.duration_since(last_reload) < debounce {
            debug!("debouncing config change event");
            continue;
        }
        last_reload = now;

        // Dispatch each changed path
        for path in &event.paths {
            dispatch_change(path, &config_manager, &agent_registry);
        }
    }
}

/// Determine whether a path belongs to the agents subsystem.
fn is_agents_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/agents/") || s.contains("\\agents\\")
}

/// Map a config filename to its `ConfigSection`, if applicable.
fn filename_to_section(filename: &str) -> Option<ConfigSection> {
    match filename {
        "models.json" => Some(ConfigSection::Models),
        "channels.json" => Some(ConfigSection::Channels),
        "gateway.json" => Some(ConfigSection::Gateway),
        "plugins.json" => Some(ConfigSection::Plugins),
        "system.json" => Some(ConfigSection::System),
        _ => None,
    }
}

/// Dispatch a single changed path to the appropriate reload method.
fn dispatch_change(path: &Path, config_manager: &ConfigManager, agent_registry: &AgentRegistry) {
    // Agent-related changes → full agent reload + registry sync
    if is_agents_path(path) {
        reload_agents_with_log(path, config_manager, agent_registry);
        return;
    }

    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f,
        None => return,
    };

    // agents.json triggers the same agent reload path
    if filename == "agents.json" {
        reload_agents_with_log(path, config_manager, agent_registry);
        return;
    }

    // Regular config section file
    if let Some(section) = filename_to_section(filename) {
        info!(
            path = %path.display(),
            section = %section,
            "config file changed, reloading section"
        );
        if let Err(e) = config_manager.reload_section(section, None) {
            warn!(error = %e, section = %section, "failed to reload config section");
        }
    }
}

/// Reload agent configs and sync the `AgentRegistry`.
///
/// Snapshots before reload; on failure, restores the previous in-memory
/// state so the daemon keeps running with the last-known-good config.
fn reload_agents_with_log(
    path: &Path,
    config_manager: &ConfigManager,
    agent_registry: &AgentRegistry,
) {
    info!(
        path = %path.display(),
        "agent config change detected, reloading agents"
    );

    let (old_agents, old_permissions) = config_manager.snapshot_agents();

    if let Err(e) = config_manager.reload_agents() {
        warn!(error = %e, "failed to reload agent configs, rolling back");
        config_manager.restore_agents(old_agents, old_permissions);
        return;
    }

    // Sync new configs into AgentRegistry
    let configs: Vec<_> = config_manager.agents().into_values().collect();
    agent_registry.reload(configs);
}

#[cfg(test)]
#[path = "reload_tests.rs"]
mod tests;
