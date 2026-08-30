//! Config hot-reload manager.
//!
//! Watches config files for changes and automatically reloads them via
//! [`ConfigManager`]. Handles debounce, file dispatch, and post-reload
//! callbacks through the [`ReloadCallback`] trait.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use tracing::{debug, info, warn};

use crate::events::ConfigChangeEvent;
use crate::manager::{ConfigLoadError, ConfigManager, ConfigSection};
use crate::providers::{ConfigProvider, CredentialsProvider, ModelsConfigData};
use crate::validators::validate_credentials;

/// Default debounce duration for file change events.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// Post-reload callback trait for daemon-level orchestration.
///
/// Implemented by the daemon crate to perform post-reload actions
/// such as agent registry sync and session provider rebuild.
pub trait ReloadCallback: Send + Sync + 'static {
    /// Called after an agent-related path change is detected.
    ///
    /// The implementor should reload agent configs and sync any
    /// registries. On failure, restore the previous in-memory state.
    fn on_agents_changed(&self, path: &Path, config_manager: &ConfigManager);

    /// Called after a permissions.json change is detected.
    ///
    /// The implementor handles lightweight permissions-only reload
    /// for the affected agent.
    fn on_permissions_changed(&self, path: &Path, config_manager: &ConfigManager);

    /// Called after a Session section reload succeeds.
    ///
    /// The implementor should rebuild the session config provider.
    fn on_session_reloaded(&self, config_manager: &ConfigManager);

    /// Called after any config file change is processed.
    ///
    /// The implementor can inspect the `section` to determine whether
    /// the change requires a restart-class action (e.g., gateway
    /// rebuild).  Default implementation is a no-op.
    fn on_config_file_changed(
        &self,
        _path: &Path,
        _section: ConfigSection,
        _config_manager: &ConfigManager,
    ) {
    }

    /// Called when config validation or parsing fails.
    ///
    /// The implementor should send an IM notification to the owner
    /// with the failure details.  Default implementation is a no-op.
    fn on_validation_failed(
        &self,
        _section: ConfigSection,
        _path: &Path,
        _error: &str,
        _config_manager: &ConfigManager,
    ) {
    }
}

/// RAII handle that keeps the filesystem watcher alive.
///
/// Dropping this handle stops the underlying watcher.
#[derive(Debug)]
#[allow(dead_code)]
pub struct WatcherHandle {
    watcher: RecommendedWatcher,
}

impl WatcherHandle {
    /// Explicitly stop watching (same as drop, but allows manual control).
    pub fn stop(self) {
        // Watcher stops when dropped
    }
}

/// Config hot-reload manager.
///
/// Watches a set of config JSON files and the `agents/` directory.
/// On change, dispatches to `ConfigManager::reload_section()` or the
/// configured [`ReloadCallback`] with debounce protection.
pub struct ConfigReloadManager {
    config_manager: Arc<ConfigManager>,
    callback: Arc<dyn ReloadCallback>,
    debounce_duration: Duration,
    #[cfg(test)]
    test_completion_tx: Option<std::sync::mpsc::Sender<()>>,
}

impl ConfigReloadManager {
    /// Create a new `ConfigReloadManager`.
    pub fn new(
        config_manager: Arc<ConfigManager>,
        callback: Arc<dyn ReloadCallback>,
        debounce_duration: Duration,
    ) -> Self {
        Self {
            config_manager,
            callback,
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

    /// Create with default debounce (500ms).
    pub fn with_defaults(
        config_manager: Arc<ConfigManager>,
        callback: Arc<dyn ReloadCallback>,
    ) -> Self {
        Self::new(config_manager, callback, DEFAULT_DEBOUNCE)
    }

    /// Clone shared references for spawning a background thread.
    fn clone_for_thread(&self) -> ConfigReloadManager {
        ConfigReloadManager {
            config_manager: Arc::clone(&self.config_manager),
            callback: Arc::clone(&self.callback),
            debounce_duration: self.debounce_duration,
            #[cfg(test)]
            test_completion_tx: None,
        }
    }

    /// Reload a single config section.
    ///
    /// Read → parse → validate → update cache. On failure, keeps
    /// the in-memory old config. File is NOT rolled back per design doc.
    pub fn reload_section(&self, section: ConfigSection) -> Result<(), ConfigLoadError> {
        let path = section.path(self.config_manager.config_dir());

        // Step 1: read file content
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                self.config_manager
                    .notify_change(ConfigChangeEvent::Failed {
                        section,
                        path: path.clone(),
                        error: e.to_string(),
                    });
                self.callback.on_validation_failed(
                    section,
                    &path,
                    &e.to_string(),
                    &self.config_manager,
                );
                return Err(ConfigLoadError::IoError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 2: parse JSON
        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                if self.config_manager.get_section_value(section).is_none() {
                    self.config_manager.block_section(section);
                }
                self.config_manager
                    .notify_change(ConfigChangeEvent::Failed {
                        section,
                        path: path.clone(),
                        error: e.to_string(),
                    });
                self.callback.on_validation_failed(
                    section,
                    &path,
                    &e.to_string(),
                    &self.config_manager,
                );
                return Err(ConfigLoadError::ParseError {
                    path,
                    error: e.to_string(),
                });
            }
        };

        // Step 3: validate
        let validate_result = if section == ConfigSection::Accounts {
            let channels_value = self
                .config_manager
                .get_section_value(ConfigSection::Channels);
            match channels_value {
                Some(channels_val) => {
                    crate::validators::validate_accounts(&value, Some(&channels_val))
                }
                None => crate::validators::validate_accounts(&value, None),
            }
        } else if section == ConfigSection::Channels {
            let cross_ref = self.config_manager.build_channels_cross_ref();
            crate::validators::validate_channels_with_refs(&value, cross_ref.as_ref())
        } else if section == ConfigSection::Models {
            let credential_providers = self.config_manager.build_models_cross_ref();
            crate::validators::validate_models_with_refs(&value, credential_providers.as_ref())
        } else {
            let validator = section.default_validator();
            validator(&value)
        };
        if let Err(msg) = validate_result {
            if self.config_manager.get_section_value(section).is_none() {
                self.config_manager.block_section(section);
            }
            self.config_manager
                .notify_change(ConfigChangeEvent::Failed {
                    section,
                    path: path.clone(),
                    error: msg.clone(),
                });
            self.callback
                .on_validation_failed(section, &path, &msg, &self.config_manager);
            return Err(ConfigLoadError::ValidationError { path, message: msg });
        }

        // Step 4: backup old in-memory value after validation passes
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

        // Step 5: success — update cache and broadcast snapshot
        // Restart-class sections (Models/Channels/Gateway) are staged
        // in the pending-restart area so the runtime cache retains the
        // old value until a gateway restart completes.
        //
        // Accounts is a special case: IM user→user ID mappings take
        // effect immediately, but bot→Agent bindings require restart.
        // When only accounts change, update the cache directly. When
        // bindings change, stage the entire new value for restart.
        if section == ConfigSection::Accounts {
            let bindings_changed = bindings_differ(&old_value, &value);
            if bindings_changed {
                self.config_manager
                    .stage_restart_value(section, path, value);
            } else {
                self.config_manager
                    .update_section_cache(section, path, value);
            }
        } else if section.is_restart_class() {
            self.config_manager
                .stage_restart_value(section, path, value);
        } else {
            self.config_manager
                .update_section_cache(section, path, value);
        }
        Ok(())
    }

    /// Reload credentials from the credentials directory.
    ///
    /// Re-reads the entire `credentials/` directory, merges in any
    /// `credential_path` references from models.json, validates each
    /// provider credential, and stages the result for restart-class
    /// effect (gateway restart required).
    ///
    /// On validation failure, the old in-memory value is retained, an
    /// IM notification is sent via the callback, and a `Failed` event
    /// is emitted.
    pub fn reload_credentials(&self) -> Result<(), ConfigLoadError> {
        let creds_dir = self
            .config_manager
            .config_dir()
            .join(CredentialsProvider::config_path());

        // Step 1+2: load from directory and merge credential_path references
        let creds_provider = self.load_and_merge_credentials(&creds_dir)?;

        // Step 3: validate each provider credential
        self.validate_all_credentials(&creds_provider, &creds_dir)?;

        // Step 4: stage for restart-class effect (gateway restart required)
        let value = serde_json::to_value(&creds_provider).map_err(|e| {
            ConfigLoadError::ValidationError {
                path: creds_dir.clone(),
                message: format!("failed to serialize credentials: {}", e),
            }
        })?;
        self.config_manager
            .stage_restart_value(ConfigSection::Credentials, creds_dir, value);
        Ok(())
    }

    /// Load credentials from the `credentials/` directory and merge any
    /// `credential_path` references found in `models.json`.
    ///
    /// Returns the merged [`CredentialsProvider`] on success. On I/O
    /// failure the error is propagated after notifying via the callback.
    fn load_and_merge_credentials(
        &self,
        creds_dir: &Path,
    ) -> Result<CredentialsProvider, ConfigLoadError> {
        // Re-read the entire credentials directory
        let mut creds_provider = match CredentialsProvider::load_from_dir(creds_dir) {
            Ok(cp) => cp,
            Err(e) => {
                self.config_manager
                    .notify_change(ConfigChangeEvent::Failed {
                        section: ConfigSection::Credentials,
                        path: creds_dir.to_path_buf(),
                        error: e.to_string(),
                    });
                self.callback.on_validation_failed(
                    ConfigSection::Credentials,
                    creds_dir,
                    &e.to_string(),
                    &self.config_manager,
                );
                return Err(ConfigLoadError::IoError {
                    path: creds_dir.to_path_buf(),
                    error: e.to_string(),
                });
            }
        };

        // Merge credential_path references from models.json
        if let Some(models_value) = self.config_manager.get_section_value(ConfigSection::Models) {
            if let Ok(models_config) =
                serde_json::from_value::<ModelsConfigData>(models_value.clone())
            {
                for (provider_id, provider_cfg) in &models_config.providers {
                    if let Some(ref rel_path) = provider_cfg.credential_path {
                        let abs_path = self.config_manager.config_dir().join(rel_path);
                        match CredentialsProvider::load_from_file(&abs_path) {
                            Ok(extra) => {
                                for (name, cred) in extra.providers {
                                    creds_provider.providers.insert(name, cred);
                                }
                            }
                            Err(e) => {
                                warn!(
                                    provider = %provider_id,
                                    path = %abs_path.display(),
                                    error = %e,
                                    "failed to load credential_path for provider"
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(creds_provider)
    }

    /// Validate every provider credential in the given [`CredentialsProvider`].
    ///
    /// Each credential is serialized and passed through [`validate_credentials`].
    /// On the first validation failure the error is propagated after notifying
    /// via the callback.
    fn validate_all_credentials(
        &self,
        creds_provider: &CredentialsProvider,
        creds_dir: &Path,
    ) -> Result<(), ConfigLoadError> {
        for (name, creds) in &creds_provider.providers {
            let value =
                serde_json::to_value(creds).map_err(|e| ConfigLoadError::ValidationError {
                    path: creds_dir.to_path_buf(),
                    message: format!("failed to serialize credential '{}': {}", name, e),
                })?;
            if let Err(msg) = validate_credentials(&value) {
                self.config_manager
                    .notify_change(ConfigChangeEvent::Failed {
                        section: ConfigSection::Credentials,
                        path: creds_dir.to_path_buf(),
                        error: format!("credential '{}' failed validation: {}", name, msg),
                    });
                self.callback.on_validation_failed(
                    ConfigSection::Credentials,
                    creds_dir,
                    &format!("credential '{}' failed validation: {}", name, msg),
                    &self.config_manager,
                );
                return Err(ConfigLoadError::ValidationError {
                    path: creds_dir.to_path_buf(),
                    message: msg,
                });
            }
        }
        Ok(())
    }

    /// Start watching config files under `config_dir`.
    pub fn watch(&mut self, config_dir: &str) -> Result<WatcherHandle, crate::ConfigError> {
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
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn create_watcher(
    tx: std::sync::mpsc::Sender<notify::Result<Event>>,
) -> Result<RecommendedWatcher, crate::ConfigError> {
    RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(Ok(event));
            }
        },
        NotifyConfig::default(),
    )
    .map_err(|e| crate::ConfigError::SchemaError(format!("Failed to create watcher: {}", e)))
}

fn register_watched_paths(
    watcher: &mut RecommendedWatcher,
    config_path: &Path,
) -> Result<(), crate::ConfigError> {
    let config_files = [
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
        "agents.json",
        "session.json",
        "memory.json",
        "skills.json",
        "media.json",
    ];
    for name in &config_files {
        let path = config_path.join(name);
        if path.exists() {
            watcher
                .watch(path.as_ref(), RecursiveMode::NonRecursive)
                .map_err(|e| {
                    crate::ConfigError::SchemaError(format!("Failed to watch {:?}: {}", path, e))
                })?;
        }
    }
    register_agents_watch(watcher, config_path)?;
    register_credentials_watch(watcher, config_path)?;
    Ok(())
}

fn register_agents_watch(
    watcher: &mut RecommendedWatcher,
    config_path: &Path,
) -> Result<(), crate::ConfigError> {
    let agents_json = config_path.join("agents.json");
    if agents_json.exists() {
        watcher
            .watch(agents_json.as_ref(), RecursiveMode::NonRecursive)
            .map_err(|e| {
                crate::ConfigError::SchemaError(format!("Failed to watch agents.json: {}", e))
            })?;
    }
    let agents_dir = config_path.parent().unwrap_or(config_path).join("agents");
    if agents_dir.exists() {
        watcher
            .watch(agents_dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| {
                crate::ConfigError::SchemaError(format!("Failed to watch agents/: {}", e))
            })?;
    }
    Ok(())
}

fn register_credentials_watch(
    watcher: &mut RecommendedWatcher,
    config_path: &Path,
) -> Result<(), crate::ConfigError> {
    let creds_dir = config_path.join("credentials");
    if creds_dir.exists() {
        watcher
            .watch(creds_dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| {
                crate::ConfigError::SchemaError(format!("Failed to watch credentials/: {}", e))
            })?;
    }
    Ok(())
}

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

fn signal_completion(tx: &Option<std::sync::mpsc::Sender<()>>) {
    if let Some(ref tx) = tx {
        let _ = tx.send(());
    }
}

/// Determine whether a path belongs to the agents subsystem.
pub fn is_agents_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/agents/") || s.contains("\\agents\\")
}

/// Determine whether a path belongs to the credentials directory.
pub fn is_credentials_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/credentials/") || s.contains("\\credentials\\")
}

/// Determine whether a path is a `permissions.json` file.
pub fn is_permissions_path(path: &Path) -> bool {
    path.file_name()
        .map(|n| n == "permissions.json")
        .unwrap_or(false)
}

/// Map a config filename to its `ConfigSection`, if applicable.
pub fn filename_to_section(filename: &str) -> Option<ConfigSection> {
    match filename {
        "models.json" => Some(ConfigSection::Models),
        "channels.json" => Some(ConfigSection::Channels),
        "gateway.json" => Some(ConfigSection::Gateway),
        "plugins.json" => Some(ConfigSection::Plugins),
        "system.json" => Some(ConfigSection::System),
        "session.json" => Some(ConfigSection::Session),
        "accounts.json" => Some(ConfigSection::Accounts),
        "agents.json" => Some(ConfigSection::Agents),
        "memory.json" => Some(ConfigSection::Memory),
        "skills.json" => Some(ConfigSection::Skills),
        "media.json" => Some(ConfigSection::Media),
        _ => None,
    }
}

/// Dispatch a single changed path to the appropriate reload method.
pub fn dispatch_change(path: &Path, manager: &ConfigReloadManager) {
    // credentials/ directory → reload credentials and stage for restart
    if is_credentials_path(path) {
        info!(
            path = %path.display(),
            "credentials file changed, triggering credentials reload"
        );
        if let Err(e) = manager.reload_credentials() {
            warn!(
                error = %e,
                "failed to reload credentials"
            );
        }
        return;
    }

    // permissions.json → lightweight permissions-only reload
    if is_agents_path(path) && is_permissions_path(path) {
        manager
            .callback
            .on_permissions_changed(path, &manager.config_manager);
        return;
    }

    // Agent-related changes → full agent reload + registry sync
    if is_agents_path(path) {
        manager
            .callback
            .on_agents_changed(path, &manager.config_manager);
        return;
    }

    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f,
        None => return,
    };

    // agents.json: standard section reload + agent directory reload
    if filename == "agents.json" {
        if let Some(section) = filename_to_section(filename) {
            info!(
                path = %path.display(),
                section = %section,
                "agents.json changed, reloading section"
            );
            if let Err(e) = manager.reload_section(section) {
                warn!(error = %e, section = %section, "failed to reload agents section");
            }
            // Also trigger agent directory reload for the AgentDirectoryProvider.
            manager
                .callback
                .on_agents_changed(path, &manager.config_manager);
        }
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
            manager
                .callback
                .on_session_reloaded(&manager.config_manager);
        }
        // Notify the callback about any config file change so it can
        // detect restart-class changes (e.g. gateway, models).
        manager
            .callback
            .on_config_file_changed(path, section, &manager.config_manager);
    }
}

/// Check whether the bot→Agent bindings differ between old and new
/// accounts config values.
///
/// Compares the `bindings` JSON array. If either side is missing the
/// field, it is treated as an empty list. Returns `true` when the
/// binding lists differ, indicating the change requires a gateway restart.
fn bindings_differ(old: &Option<serde_json::Value>, new: &serde_json::Value) -> bool {
    let old_bindings = old
        .as_ref()
        .and_then(|v| v.get("bindings"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let new_bindings = new
        .get("bindings")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    old_bindings != new_bindings
}

#[cfg(test)]
#[path = "reload_manager_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "reload_staging_tests.rs"]
mod reload_staging_tests;
