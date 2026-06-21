//! Config Hot Reload Manager
//!
//! Watches config files for changes and automatically reloads them via
//! `ConfigManager`. Handles debounce, file dispatch, and agent directory
//! changes (snapshot → reload → `AgentRegistry::sync`).

use std::path::Path;
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
    /// Optional channel for test completion signals.
    #[cfg(test)]
    test_completion_tx: Option<std::sync::mpsc::Sender<()>>,
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
            #[cfg(test)]
            test_completion_tx: None,
        }
    }

    /// Set a completion signal channel for tests.
    #[cfg(test)]
    pub fn set_test_completion(&mut self, tx: std::sync::mpsc::Sender<()>) {
        self.test_completion_tx = Some(tx);
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
    pub fn watch(&mut self, config_dir: &str) -> Result<WatcherHandle, super::ConfigError> {
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
        let mut watcher = create_watcher(tx)?;
        let config_path = Path::new(config_dir);
        register_watched_paths(&mut watcher, config_path)?;
        #[cfg(test)]
        let completion_tx = self.test_completion_tx.take();
        #[cfg(not(test))]
        let completion_tx: Option<std::sync::mpsc::Sender<()>> = None;
        spawn_reload_loop(
            rx,
            &self.config_manager,
            &self.agent_registry,
            self.debounce_duration,
            completion_tx,
        );
        info!(config_dir = config_dir, "config hot-reload watcher started");
        Ok(WatcherHandle { watcher })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Create a `RecommendedWatcher` with an mpsc sender for file change events.
fn create_watcher(
    tx: std::sync::mpsc::Sender<notify::Result<Event>>,
) -> Result<RecommendedWatcher, super::ConfigError> {
    RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(Ok(event));
            }
        },
        NotifyConfig::default(),
    )
    .map_err(|e| super::ConfigError::SchemaError(format!("Failed to create watcher: {}", e)))
}

/// Register all watched paths (config files, agents.json, agents/ dir).
fn register_watched_paths(
    watcher: &mut RecommendedWatcher,
    config_path: &Path,
) -> Result<(), super::ConfigError> {
    let config_files = [
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
    ];
    for name in &config_files {
        let path = config_path.join(name);
        if path.exists() {
            watcher
                .watch(path.as_ref(), RecursiveMode::NonRecursive)
                .map_err(|e| {
                    super::ConfigError::SchemaError(format!("Failed to watch {:?}: {}", path, e))
                })?;
        }
    }
    register_agents_watch(watcher, config_path)?;
    Ok(())
}

/// Register watchers for agents.json and agents/ directory.
fn register_agents_watch(
    watcher: &mut RecommendedWatcher,
    config_path: &Path,
) -> Result<(), super::ConfigError> {
    let agents_json = config_path.join("agents.json");
    if agents_json.exists() {
        watcher
            .watch(agents_json.as_ref(), RecursiveMode::NonRecursive)
            .map_err(|e| {
                super::ConfigError::SchemaError(format!("Failed to watch agents.json: {}", e))
            })?;
    }
    let agents_dir = config_path.join("agents");
    if agents_dir.exists() {
        watcher
            .watch(agents_dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| {
                super::ConfigError::SchemaError(format!("Failed to watch agents/: {}", e))
            })?;
    }
    Ok(())
}

/// Spawn the debounce + dispatch loop on a background thread.
fn spawn_reload_loop(
    rx: std::sync::mpsc::Receiver<notify::Result<Event>>,
    config_manager: &Arc<ConfigManager>,
    agent_registry: &Arc<AgentRegistry>,
    debounce: Duration,
    completion_tx: Option<std::sync::mpsc::Sender<()>>,
) {
    let cm = Arc::clone(config_manager);
    let ar = Arc::clone(agent_registry);
    std::thread::spawn(move || {
        run_reload_loop(rx, cm, ar, debounce, completion_tx);
    });
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
    completion_tx: Option<std::sync::mpsc::Sender<()>>,
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

        // Signal test completion if a sender is available
        if let Some(ref tx) = completion_tx {
            let _ = tx.send(());
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
        let validator = section.default_validator();
        if let Err(e) = config_manager.reload_section(section, Some(&*validator)) {
            warn!(error = %e, section = %section, "failed to reload config section");
        }
    }
}

/// Reload agent configs and sync the `AgentRegistry`.
///
/// Snapshots before reload; on failure, restores the previous in-memory
/// state so the daemon keeps running with the last-known-good config.
/// Additionally, agent config files (agents.json, agents/*.json) are
/// backed up before reload and rolled back on failure to keep the
/// on-disk state consistent.
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

    // Backup agent config files before reload so we can rollback on failure
    let agents_json = config_manager.config_dir.join("config").join("agents.json");
    let _ = config_manager.backup_manager().backup(&agents_json);

    let agents_dir = config_manager.config_dir.join("agents");
    if agents_dir.exists() {
        let _ = backup_agents_dir(&agents_dir, config_manager.backup_manager());
    }

    if let Err(e) = config_manager.reload_agents() {
        warn!(error = %e, "failed to reload agent configs, rolling back");
        config_manager.restore_agents(old_agents, old_permissions);

        // Rollback disk files to last known good state
        let _ = config_manager.backup_manager().rollback(&agents_json);
        if agents_dir.exists() {
            let _ = rollback_agents_dir(&agents_dir, config_manager.backup_manager());
        }
        return;
    }

    // Sync new configs into AgentRegistry
    let configs: Vec<_> = config_manager.agents().into_values().collect();
    agent_registry.reload(configs);
}

/// Backup all agent config files under `agents/` directory.
///
/// Iterates `.json` files in the directory and backs each one up.
fn backup_agents_dir(
    agents_dir: &Path,
    backup_manager: &super::manager::SafeBackupManager,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(agents_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "json") {
            let _ = backup_manager.backup(&path);
        }
    }
    Ok(())
}

/// Rollback all agent config files under `agents/` directory.
///
/// Iterates `.json` files in the directory and rolls each one back.
fn rollback_agents_dir(
    agents_dir: &Path,
    backup_manager: &super::manager::SafeBackupManager,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(agents_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "json") {
            let _ = backup_manager.rollback(&path);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "reload_tests.rs"]
mod tests;
