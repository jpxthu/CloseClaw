//! Config Hot Reload Manager
//!
//! Watches config files for changes and automatically reloads them via
//! `ConfigManager`. Handles debounce, file dispatch, and agent directory
//! changes (snapshot → reload → `AgentRegistry::sync`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use tracing::{debug, info, warn};

use crate::agent::registry::AgentRegistry;
use closeclaw_config::events::ConfigChangeEvent;
use closeclaw_config::manager::{ConfigLoadError, ConfigManager, ConfigSection};

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
/// On change, dispatches to `ConfigReloadManager::reload_section()` or
/// `ConfigReloadManager::reload_agents_with_log()` with debounce protection.
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

    /// Clone shared references for spawning a background thread.
    ///
    /// Clones the `Arc` references (cheap) so the background thread
    /// can call `ConfigReloadManager` methods directly.
    fn clone_for_thread(&self) -> ConfigReloadManager {
        ConfigReloadManager {
            config_manager: Arc::clone(&self.config_manager),
            agent_registry: Arc::clone(&self.agent_registry),
            debounce_duration: self.debounce_duration,
            #[cfg(test)]
            test_completion_tx: None,
        }
    }

    /// Reload a single config section: read file → parse → validate → update cache.
    ///
    /// On success, updates the in-memory cache and broadcasts a snapshot.
    /// On failure, rolls back the file and emits a Failed event.
    /// Corresponds to the design doc data flow: ConfigReloadManager drives
    /// the reload orchestration (read → parse → validate → update).
    pub fn reload_section(&self, section: ConfigSection) -> Result<(), ConfigLoadError> {
        let path = section.path(self.config_manager.config_dir());

        // Step 1: read file content
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                self.config_manager
                    .notify_change(ConfigChangeEvent::Failed {
                        section,
                        error: e.to_string(),
                    });
                return Err(ConfigLoadError::IoError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 2: backup old in-memory value before replacing
        let old_value = self.config_manager.get_section_value(section);
        if let Some(ref old) = old_value {
            let old_json = serde_json::to_string(old).unwrap_or_default();
            if let Err(e) = self
                .config_manager
                .backup_manager()
                .backup_with_content(&path, old_json.as_bytes())
            {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to backup config content before reload"
                );
            }
        }

        // Step 3: parse JSON
        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                let _ = self.config_manager.rollback_file(&path);
                self.config_manager
                    .notify_change(ConfigChangeEvent::Failed {
                        section,
                        error: e.to_string(),
                    });
                return Err(ConfigLoadError::ParseError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 4: validate with section's default validator
        let validator = section.default_validator();
        if let Err(msg) = validator(&value) {
            let _ = self.config_manager.rollback_file(&path);
            self.config_manager
                .notify_change(ConfigChangeEvent::Failed {
                    section,
                    error: msg.clone(),
                });
            return Err(ConfigLoadError::ValidationError { path, message: msg });
        }

        // Step 5: success — update cache and broadcast snapshot
        self.config_manager.update_section_cache(section, value);
        Ok(())
    }

    /// Reload the session config provider from disk.
    ///
    /// Delegates to `ConfigManager::reload_session_provider()`.
    pub(crate) fn reload_session_provider(&self) {
        self.config_manager.reload_session_provider();
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
    pub fn watch(
        &mut self,
        config_dir: &str,
    ) -> Result<WatcherHandle, closeclaw_config::ConfigError> {
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
        let mut watcher = create_watcher(tx)?;
        let config_path = Path::new(config_dir);
        register_watched_paths(&mut watcher, config_path)?;
        #[cfg(test)]
        let completion_tx = self.test_completion_tx.take();
        #[cfg(not(test))]
        let completion_tx: Option<std::sync::mpsc::Sender<()>> = None;
        let manager_clone = self.clone_for_thread();
        spawn_reload_loop(rx, manager_clone, self.debounce_duration, completion_tx);
        info!(config_dir = config_dir, "config hot-reload watcher started");
        Ok(WatcherHandle { watcher })
    }

    /// Reload agent configs and sync the `AgentRegistry`.
    ///
    /// Snapshots before reload; on failure, restores the previous in-memory
    /// state so the daemon keeps running with the last-known-good config.
    /// Additionally, agent config files (agents.json, agents/*.json) are
    /// backed up before reload and rolled back on failure to keep the
    /// on-disk state consistent.
    fn reload_agents_with_log(&self, path: &Path) {
        info!(
            path = %path.display(),
            "agent config change detected, reloading agents"
        );

        let (old_agents, old_permissions) = self.config_manager.snapshot_agents();

        // Backup agent config files before reload so we can rollback on failure
        let agents_json = self.config_manager.config_dir().join("agents.json");
        let _ = self.config_manager.backup_manager().backup(&agents_json);

        let agents_dir = self
            .config_manager
            .config_dir()
            .parent()
            .unwrap_or(self.config_manager.config_dir())
            .join("agents");
        if agents_dir.exists() {
            for_each_agent_json(&agents_dir, |p| {
                let _ = self.config_manager.backup_manager().backup(p);
            });
        }

        if let Err(e) = self.config_manager.reload_agents() {
            warn!(error = %e, "failed to reload agent configs, rolling back");
            self.config_manager
                .restore_agents(old_agents, old_permissions);

            // Rollback disk files to last known good state
            let _ = self.config_manager.backup_manager().rollback(&agents_json);
            if agents_dir.exists() {
                for_each_agent_json(&agents_dir, |p| {
                    let _ = self.config_manager.backup_manager().rollback(p);
                });
            }
            return;
        }

        // Sync new configs into AgentRegistry
        let configs: Vec<_> = self.config_manager.agents().into_values().collect();
        self.agent_registry.reload(configs);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Create a `RecommendedWatcher` with an mpsc sender for file change events.
fn create_watcher(
    tx: std::sync::mpsc::Sender<notify::Result<Event>>,
) -> Result<RecommendedWatcher, closeclaw_config::ConfigError> {
    RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(Ok(event));
            }
        },
        NotifyConfig::default(),
    )
    .map_err(|e| {
        closeclaw_config::ConfigError::SchemaError(format!("Failed to create watcher: {}", e))
    })
}

/// Register all watched paths (config files, agents.json, agents/ dir).
fn register_watched_paths(
    watcher: &mut RecommendedWatcher,
    config_path: &Path,
) -> Result<(), closeclaw_config::ConfigError> {
    let config_files = [
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "session.json",
    ];
    for name in &config_files {
        let path = config_path.join(name);
        if path.exists() {
            watcher
                .watch(path.as_ref(), RecursiveMode::NonRecursive)
                .map_err(|e| {
                    closeclaw_config::ConfigError::SchemaError(format!(
                        "Failed to watch {:?}: {}",
                        path, e
                    ))
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
) -> Result<(), closeclaw_config::ConfigError> {
    let agents_json = config_path.join("agents.json");
    if agents_json.exists() {
        watcher
            .watch(agents_json.as_ref(), RecursiveMode::NonRecursive)
            .map_err(|e| {
                closeclaw_config::ConfigError::SchemaError(format!(
                    "Failed to watch agents.json: {}",
                    e
                ))
            })?;
    }
    let agents_dir = config_path.parent().unwrap_or(config_path).join("agents");
    if agents_dir.exists() {
        watcher
            .watch(agents_dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| {
                closeclaw_config::ConfigError::SchemaError(format!(
                    "Failed to watch agents/: {}",
                    e
                ))
            })?;
    }
    Ok(())
}

/// Spawn the debounce + dispatch loop on a background thread.
///
/// The `ConfigReloadManager` is cloned (Arc refs are shared) so the
/// background thread can call its methods directly.
fn spawn_reload_loop(
    rx: std::sync::mpsc::Receiver<notify::Result<Event>>,
    manager: ConfigReloadManager,
    debounce: Duration,
    completion_tx: Option<std::sync::mpsc::Sender<()>>,
) {
    std::thread::spawn(move || {
        run_reload_loop(rx, manager, debounce, completion_tx);
    });
}

// ---------------------------------------------------------------------------
// Internal event loop
// ---------------------------------------------------------------------------

/// Collect relevant paths from a file change event into the pending set.
/// Filters out non-create/modify/remove events. Paths are inserted into
/// `pending_paths` (a `HashSet` ensures deduplication).
fn collect_event_paths(event: Event, pending_paths: &mut HashSet<PathBuf>) {
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {}
        _ => return,
    }
    for path in event.paths {
        pending_paths.insert(path);
    }
    debug!(
        count = pending_paths.len(),
        "collected config change events"
    );
}

/// Background loop: receive file change events, debounce, and dispatch.
///
/// Uses a "collect-merge-execute" debounce strategy:
/// - During the debounce window, changed paths are collected into a
///   `HashSet<PathBuf>` (deduplicating repeated paths).
/// - When the window expires (no new events for `debounce` duration),
///   all collected paths are dispatched in a single batch.
fn run_reload_loop(
    rx: std::sync::mpsc::Receiver<notify::Result<Event>>,
    manager: ConfigReloadManager,
    debounce: Duration,
    completion_tx: Option<std::sync::mpsc::Sender<()>>,
) {
    let mut pending_paths: HashSet<PathBuf> = HashSet::new();

    loop {
        match rx.recv_timeout(debounce) {
            Ok(event_result) => match event_result {
                Ok(event) => collect_event_paths(event, &mut pending_paths),
                Err(e) => warn!("config watcher event error: {}", e),
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if pending_paths.is_empty() {
                    continue;
                }
                dispatch_pending_batch(&pending_paths, &manager);
                pending_paths.clear();
                signal_completion(&completion_tx);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                dispatch_pending_batch(&pending_paths, &manager);
                signal_completion(&completion_tx);
                break;
            }
        }
    }
}

/// Dispatch all paths in a batch and signal test completion.
fn dispatch_pending_batch(paths: &HashSet<PathBuf>, manager: &ConfigReloadManager) {
    if paths.is_empty() {
        return;
    }
    info!(
        count = paths.len(),
        "debounce window closed, dispatching batch"
    );
    for path in paths {
        dispatch_change(path, manager);
    }
}

/// Send test completion signal if a sender is available.
fn signal_completion(tx: &Option<std::sync::mpsc::Sender<()>>) {
    if let Some(ref tx) = tx {
        let _ = tx.send(());
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
        "session.json" => Some(ConfigSection::Session),
        _ => None,
    }
}

/// Dispatch a single changed path to the appropriate reload method.
///
/// This delegates to `ConfigReloadManager` methods, which drive the
/// reload orchestration per the design doc (ConfigReloadManager → ConfigManager → SessionManager).
fn dispatch_change(path: &Path, manager: &ConfigReloadManager) {
    // Agent-related changes → full agent reload + registry sync
    if is_agents_path(path) {
        manager.reload_agents_with_log(path);
        return;
    }

    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f,
        None => return,
    };

    // agents.json triggers the same agent reload path
    if filename == "agents.json" {
        manager.reload_agents_with_log(path);
        return;
    }

    // Regular config section file
    if let Some(section) = filename_to_section(filename) {
        info!(
            path = %path.display(),
            section = %section,
            "config file changed, reloading section"
        );
        if let Err(e) = manager.reload_section(section) {
            warn!(error = %e, section = %section, "failed to reload config section");
        } else if section == ConfigSection::Session {
            // Session section reloaded successfully — update session_provider.
            manager.reload_session_provider();
        }
    }
}

/// Iterate over `.json` files in the agents directory and apply `f` to each.
///
/// Used to factor out the shared backup/rollback logic for `agents/` dir.
fn for_each_agent_json<F>(agents_dir: &Path, f: F)
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

#[cfg(test)]
#[path = "reload_tests.rs"]
mod tests;
