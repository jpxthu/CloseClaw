//! Config Manager — unified config management entry point
//!
//! Provides atomic write, backup integration, and unified access
//! to all JSON config files under the config/ directory.

use chrono::{DateTime, Local};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use tokio::sync::broadcast;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::backup::SafeBackupManager;
use crate::events::{ConfigChangeBroadcaster, ConfigChangeEvent};

/// Snapshot of all config sections at a point in time.
///
/// Broadcast via `ConfigManager` on every successful reload so that
/// downstream components (e.g. `SessionManager`) can swap to the
/// latest config without holding a lock on `ConfigManager`.
pub type ConfigSnapshot = Arc<HashMap<ConfigSection, serde_json::Value>>;
use crate::agents::LazyAgentPermissions;
use crate::providers::{ConfigProvider, CredentialsProvider, ModelsConfigData};
use crate::session::{JsonSessionConfigProvider, SessionConfigProvider};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Error loading a configuration file.
#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("config directory not found: {0}")]
    ConfigDirNotFound(PathBuf),

    #[error("config file not found: {0}")]
    ConfigFileNotFound(PathBuf),

    #[error("failed to parse config file {path}: {error}")]
    ParseError { path: PathBuf, error: String },

    #[error("config validation failed for {path}: {message}")]
    ValidationError { path: PathBuf, message: String },

    #[error("backup not found for {0}")]
    BackupNotFound(PathBuf),

    #[error("I/O error loading {path}: {error}")]
    IoError { path: PathBuf, error: String },
}

impl From<io::Error> for ConfigLoadError {
    fn from(e: io::Error) -> Self {
        ConfigLoadError::IoError {
            path: PathBuf::new(),
            error: e.to_string(),
        }
    }
}

/// Error writing a configuration file.
#[derive(Debug, Error)]
pub enum ConfigWriteError {
    #[error("validation failed for {0}: {1}")]
    ValidationFailed(String, String),

    #[error("backup failed for {path}: {error}")]
    BackupFailed { path: PathBuf, error: String },

    #[error("write failed for {path}: {error}")]
    WriteFailed { path: PathBuf, error: String },

    #[error("config file not found: {0}")]
    FileNotFound(PathBuf),
}

impl From<io::Error> for ConfigWriteError {
    fn from(e: io::Error) -> Self {
        ConfigWriteError::WriteFailed {
            path: PathBuf::new(),
            error: e.to_string(),
        }
    }
}

/// Validation error for a config file.
#[derive(Debug)]
pub struct ConfigValidationError {
    pub path: PathBuf,
    pub message: String,
}

impl std::fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "validation failed for {}: {}",
            self.path.display(),
            self.message
        )
    }
}

impl std::error::Error for ConfigValidationError {}

// ---------------------------------------------------------------------------
// Atomic write
// ---------------------------------------------------------------------------

/// Write content to a file atomically: write to a temp file, fsync,
/// then rename to the target path. Cleans up the temp file on failure.
///
/// If `file_permissions` is `Some(mode)`, the target file's permission
/// is set to `mode` (octal, e.g. `0o600`) after the rename.
pub fn write_atomically(
    path: &Path,
    content: &[u8],
    file_permissions: Option<u32>,
) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;

    let tmp_name = format!(".tmp.{}", Uuid::new_v4());
    let tmp_path = parent.join(tmp_name);

    let mut file = File::create(&tmp_path)?;

    if let Err(e) = file.write_all(content) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = file.sync_all() {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Sync parent directory to make the rename durable.
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }

    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Set file permissions after rename to ensure the final file has
    // the correct mode. This is done post-rename so the rename itself
    // remains atomic.
    if let Some(mode) = file_permissions {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Config section
// ---------------------------------------------------------------------------

/// Represents a configuration section backed by a single JSON file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigSection {
    Models,
    Channels,
    Gateway,
    Plugins,
    System,
    Session,
    Credentials,
    Accounts,
    Memory,
}

impl ConfigSection {
    /// Returns the filename associated with this section.
    pub fn filename(&self) -> &'static str {
        match self {
            ConfigSection::Models => "models.json",
            ConfigSection::Channels => "channels.json",
            ConfigSection::Gateway => "gateway.json",
            ConfigSection::Plugins => "plugins.json",
            ConfigSection::System => "system.json",
            ConfigSection::Session => "session.json",
            ConfigSection::Credentials => "credentials/",
            ConfigSection::Accounts => "accounts.json",
            ConfigSection::Memory => "memory.json",
        }
    }

    /// Returns the absolute path to this section's file relative to config_dir.
    pub fn path(&self, config_dir: &Path) -> PathBuf {
        config_dir.join(self.filename())
    }
}

impl std::fmt::Display for ConfigSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.filename())
    }
}

// ---------------------------------------------------------------------------
// Config info
// ---------------------------------------------------------------------------

/// Metadata about a configuration file.
#[derive(Debug, Clone)]
pub struct ConfigInfo {
    /// Absolute path to the config file.
    pub path: String,
    /// Config file format version (from the JSON content).
    pub version: String,
    /// Last modified timestamp (None if the file hasn't been read yet).
    pub last_modified: Option<DateTime<Local>>,
}

// ---------------------------------------------------------------------------
// ConfigManager
// ---------------------------------------------------------------------------

/// Unified configuration management entry point.
/// Provides atomic write, backup integration, and unified access to all
/// JSON config files under the config/ directory.
///
/// # Design
/// Each configuration section (models, channels, gateway, etc.) is stored
/// as a separate JSON file. ConfigManager loads all sections into memory
/// and provides:
/// - `load()`: load all sections from disk
/// - `update()`: validate + backup + atomic write + update memory
/// - `section()`: read-only access to a section's JSON value
/// - `list_configs()`: metadata about all config files
pub struct ConfigManager {
    /// Base directory containing all config files.
    pub config_dir: PathBuf,
    /// Thread-safe backup manager.
    pub backup_manager: SafeBackupManager,
    /// In-memory cache of all loaded config sections.
    pub(crate) sections: RwLock<HashMap<ConfigSection, serde_json::Value>>,
    /// Loaded credentials provider (from config/credentials/ directory).
    credentials_provider: RwLock<CredentialsProvider>,
    /// Loaded session config provider (from config/session.json).
    pub session_provider: RwLock<Option<Arc<dyn SessionConfigProvider>>>,
    /// Resolved agent configurations (loaded from two-level directories).
    pub agents: RwLock<HashMap<String, crate::agents::ResolvedAgentConfig>>,
    /// Lazy-loaded agent permissions (accessed on demand via get()).
    pub agent_permissions: Arc<LazyAgentPermissions>,
    /// Optional project root for loading project-level agents.json.
    pub(crate) repo_root: RwLock<Option<PathBuf>>,
    /// Broadcast channel for config change events.
    event_broadcaster: ConfigChangeBroadcaster,
    /// Broadcast channel for config snapshots after each successful reload.
    snapshot_tx: broadcast::Sender<ConfigSnapshot>,
    /// Sections that are blocked because they had no previous value
    /// and the initial load/reload failed. Blocked sections return
    /// `None` from `get_section_value` and are unblocked on next
    /// successful reload.
    blocked_sections: RwLock<HashSet<ConfigSection>>,
}

impl ConfigManager {
    /// Create a new ConfigManager for the given config directory.
    ///
    /// The backup manager will store backups in `<config_dir>/.backups/`.
    pub fn new(config_dir: PathBuf) -> io::Result<Self> {
        let backup_dir = config_dir.join(".backups");
        let backup_manager =
            SafeBackupManager::new(crate::backup::BackupManager::new(backup_dir, 10)?);
        // Compute agents root before config_dir is moved into the struct.
        let agents_root = config_dir.parent().unwrap_or(&config_dir).to_path_buf();
        Ok(Self {
            config_dir,
            backup_manager,
            sections: RwLock::new(HashMap::new()),
            credentials_provider: RwLock::new(CredentialsProvider::default()),
            session_provider: RwLock::new(None),
            agents: RwLock::new(HashMap::new()),
            agent_permissions: Arc::new(LazyAgentPermissions::new(agents_root)),
            repo_root: RwLock::new(None),
            event_broadcaster: ConfigChangeBroadcaster::new(),
            snapshot_tx: broadcast::channel(16).0,
            blocked_sections: RwLock::new(HashSet::new()),
        })
    }

    /// Get a reference to the shared backup manager.
    ///
    /// Used by `ConfigReloadManager` to backup/rollback agent config files
    /// without creating a separate `BackupManager` instance.
    pub fn backup_manager(&self) -> &SafeBackupManager {
        &self.backup_manager
    }

    /// Get a reference to the config directory.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Rollback a file to its most recent backup.
    pub fn rollback_file(&self, path: &Path) -> Result<(), io::Error> {
        self.backup_manager.rollback(path).map(|_| ())
    }

    /// Load all configuration sections from disk into memory.
    ///
    /// Returns `Ok(())` if all mandatory config files are loaded successfully.
    /// Returns `Err(ConfigLoadError::ConfigFileNotFound)` if a mandatory file is missing.
    /// Returns `Err(ConfigLoadError::ConfigDirNotFound)` if the config directory doesn't exist.
    pub fn load(&self) -> Result<(), ConfigLoadError> {
        if !self.config_dir.exists() {
            return Err(ConfigLoadError::ConfigDirNotFound(self.config_dir.clone()));
        }

        // The 5 mandatory config files (Credentials is a directory,
        // Session is optional with defaults — both handled separately)
        let mandatory_sections = [
            ConfigSection::Models,
            ConfigSection::Channels,
            ConfigSection::Gateway,
            ConfigSection::Plugins,
            ConfigSection::System,
            ConfigSection::Accounts,
        ];

        let mut sections = self
            .sections
            .write()
            .expect("RwLock for config sections was poisoned");

        for section in mandatory_sections {
            let path = section.path(&self.config_dir);
            if !path.exists() {
                return Err(ConfigLoadError::ConfigFileNotFound(path));
            }

            let content = fs::read_to_string(&path).map_err(|e| ConfigLoadError::IoError {
                path: path.clone(),
                error: e.to_string(),
            })?;

            let value: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_parse_err) => {
                    // Try rollback + retry before reporting the error
                    match self.try_rollback_and_retry(&path, section, &mut sections) {
                        Ok(()) => continue,
                        Err(e) => return Err(e),
                    }
                }
            };

            // Business validation: reuse the same validators used by hot-reload.
            // For Accounts, pass the channels config for cross-reference validation.
            let validate_result = if section == ConfigSection::Accounts {
                let channels_value = sections.get(&ConfigSection::Channels).cloned();
                match channels_value {
                    Some(channels_val) => {
                        crate::validators::validate_accounts(&value, Some(&channels_val))
                    }
                    None => crate::validators::validate_accounts(&value, None),
                }
            } else {
                let validate = crate::validators::for_section(section);
                validate(&value)
            };
            if let Err(msg) = validate_result {
                warn!(
                    section = %section,
                    error = %msg,
                    "config business validation failed, attempting rollback"
                );
                match self.try_rollback_and_retry(&path, section, &mut sections) {
                    Ok(()) => continue,
                    Err(e) => return Err(e),
                }
            }

            sections.insert(section, value);
        }

        // Load session config (optional — absent file uses defaults).
        let session_path = ConfigSection::Session.path(&self.config_dir);
        let session_provider: Arc<dyn SessionConfigProvider> =
            match JsonSessionConfigProvider::new(&session_path) {
                Ok(provider) => {
                    info!(
                        path = %session_path.display(),
                        "session config loaded from {}",
                        session_path.display()
                    );
                    // Also store raw JSON in sections for hot-reload consistency.
                    if let Ok(content) = fs::read_to_string(&session_path) {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                            sections.insert(ConfigSection::Session, value);
                        }
                    }
                    Arc::new(provider)
                }
                Err(e) => {
                    warn!(
                        path = %session_path.display(),
                        error = %e,
                        "failed to load session config, using defaults"
                    );
                    Arc::new(JsonSessionConfigProvider::default())
                }
            };
        *self.session_provider.write().expect("RwLock poisoned") = Some(session_provider);

        // Load credentials from config/credentials/ directory.
        let creds_dir = self.config_dir.join(CredentialsProvider::config_path());
        let mut creds_provider = match CredentialsProvider::load_from_dir(&creds_dir) {
            Ok(cp) => cp,
            Err(e) => {
                warn!(
                    "failed to load credentials from '{}': {}",
                    creds_dir.display(),
                    e
                );
                CredentialsProvider::default()
            }
        };

        // Load additional credentials via credential_path from models.json.
        // Each provider in models.json may specify a credential_path pointing to a
        // credential file.  Resolve it relative to config_dir and merge into the
        // credential set (credential_path takes priority over convention-directory
        // entries).
        if let Some(models_value) = sections.get(&ConfigSection::Models) {
            if let Ok(models_config) =
                serde_json::from_value::<ModelsConfigData>(models_value.clone())
            {
                for (provider_id, provider_cfg) in &models_config.providers {
                    if let Some(ref rel_path) = provider_cfg.credential_path {
                        let abs_path = self.config_dir.join(rel_path);
                        match CredentialsProvider::load_from_file(&abs_path) {
                            Ok(extra) => {
                                for (name, cred) in extra.providers {
                                    // credential_path is the explicit reference
                                    // and takes priority over the convention
                                    // directory.
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

        *self.credentials_provider.write().expect("RwLock poisoned") = creds_provider.clone();
        // Store as JSON value in sections (may be empty/default if dir is absent)
        if let Ok(json) = serde_json::to_value(&creds_provider) {
            sections.insert(ConfigSection::Credentials, json);
        }

        // Load global memory config (optional — absent file uses defaults).
        let memory_path = ConfigSection::Memory.path(&self.config_dir);
        if memory_path.exists() {
            match fs::read_to_string(&memory_path) {
                Ok(content) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                        sections.insert(ConfigSection::Memory, value);
                        info!(
                            path = %memory_path.display(),
                            "global memory config loaded"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        path = %memory_path.display(),
                        error = %e,
                        "failed to read memory.json, using defaults"
                    );
                }
            }
        } else {
            info!("memory.json not found, using defaults");
        }

        // Cross-validate credentials against models.json references.
        if let Some(models_value) = sections.get(&ConfigSection::Models) {
            match serde_json::from_value::<ModelsConfigData>(models_value.clone()) {
                Ok(models_config) => {
                    if let Err(e) =
                        creds_provider.validate_model_references(&models_config, &self.config_dir)
                    {
                        warn!(
                            error = %e,
                            "credentials-models cross-validation warning"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "failed to parse models.json for credentials cross-validation"
                    );
                }
            }
        }

        drop(sections);

        // Load agent configurations (non-fatal if agents.json is absent)
        if let Err(e) = self.load_agents(None) {
            warn!("failed to load agent configs: {}", e);
        }

        Ok(())
    }

    /// Attempt to rollback a corrupted config file and retry loading.
    /// Returns Ok(()) if rollback succeeded and retry loading worked.
    /// Returns Err(ConfigLoadError::ParseError) if rollback failed or retry still fails.
    fn try_rollback_and_retry(
        &self,
        path: &Path,
        section: ConfigSection,
        sections: &mut HashMap<ConfigSection, serde_json::Value>,
    ) -> Result<(), ConfigLoadError> {
        // Try to find a backup
        match self.backup_manager.find_latest_backup(path) {
            Ok(backup_path) => {
                if self.backup_manager.rollback(path).is_ok() {
                    let retry_content = fs::read_to_string(path).ok();
                    if let Some(retry_content) = retry_content {
                        if let Ok(retry_value) =
                            serde_json::from_str::<serde_json::Value>(&retry_content)
                        {
                            warn!("已用备份恢复 {}, 备份来源: {:?}", section, backup_path);
                            sections.insert(section, retry_value);
                            return Ok(());
                        }
                    }
                }
                // Retry failed
                error!("配置文件 {} 恢复后仍无法解析，daemon 无法启动", section);
                Err(ConfigLoadError::ParseError {
                    path: path.to_path_buf(),
                    error: "rollback succeeded but file still unparseable".to_string(),
                })
            }
            Err(_) => {
                // No backup found
                error!("配置文件 {} 损坏且无备份，daemon 无法启动", section);
                Err(ConfigLoadError::ParseError {
                    path: path.to_path_buf(),
                    error: "no backup available".to_string(),
                })
            }
        }
    }

    /// Update a configuration section.
    ///
    /// Flow: validate → backup current content → atomic write → update in-memory cache.
    ///
    /// If validation fails, no file is written.
    /// If backup fails, no file is written (write_atomically is not called).
    pub fn update(
        &self,
        section: ConfigSection,
        new_value: serde_json::Value,
        validator: impl FnOnce(&serde_json::Value) -> Result<(), ConfigValidationError>,
    ) -> Result<(), ConfigWriteError> {
        // Step 1: validate
        if let Err(e) = validator(&new_value) {
            return Err(ConfigWriteError::ValidationFailed(
                section.to_string(),
                e.message.clone(),
            ));
        }

        let path = section.path(&self.config_dir);

        // Step 2: backup current content (if file exists)
        if path.exists() {
            let current_content = fs::read(&path).map_err(|e| ConfigWriteError::BackupFailed {
                path: path.clone(),
                error: e.to_string(),
            })?;
            self.backup_manager
                .backup_with_content(&path, &current_content)
                .map_err(|e: io::Error| ConfigWriteError::BackupFailed {
                    path: path.clone(),
                    error: e.to_string(),
                })?;
        }

        // Step 3: atomic write
        let content =
            serde_json::to_vec_pretty(&new_value).map_err(|e| ConfigWriteError::WriteFailed {
                path: path.clone(),
                error: e.to_string(),
            })?;

        let permissions = if section == ConfigSection::Credentials {
            Some(0o600)
        } else {
            None
        };
        write_atomically(&path, &content, permissions).map_err(|e| {
            ConfigWriteError::WriteFailed {
                path: path.clone(),
                error: e.to_string(),
            }
        })?;

        // Step 4: update in-memory cache
        let mut sections = self
            .sections
            .write()
            .expect("RwLock for config sections was poisoned");
        sections.insert(section, new_value);

        Ok(())
    }

    /// Get a read-only clone of a configuration section's JSON value.
    ///
    /// Returns `None` if the section has not been loaded.
    pub fn section(&self, section: ConfigSection) -> Option<serde_json::Value> {
        self.get_section_value(section)
    }

    /// Get a single section value from the in-memory cache.
    ///
    /// Returns `None` if the section has not been loaded or is blocked.
    pub fn get_section_value(&self, section: ConfigSection) -> Option<serde_json::Value> {
        if self.is_blocked(section) {
            return None;
        }
        self.sections
            .read()
            .expect("RwLock for config sections was poisoned")
            .get(&section)
            .cloned()
    }

    /// Check if a section is blocked (no old value + failed load).
    pub fn is_blocked(&self, section: ConfigSection) -> bool {
        self.blocked_sections
            .read()
            .expect("RwLock for blocked_sections was poisoned")
            .contains(&section)
    }

    /// Block a section because it had no old value and the reload failed.
    ///
    /// A blocked section's `get_section_value` returns `None`, which
    /// signals downstream components that the config is unavailable.
    pub fn block_section(&self, section: ConfigSection) {
        warn!(
            section = %section,
            "blocking config section: no old value and reload failed"
        );
        self.blocked_sections
            .write()
            .expect("RwLock for blocked_sections was poisoned")
            .insert(section);
    }

    /// Unblock a section. Called after a successful reload.
    pub fn unblock_section(&self, section: ConfigSection) {
        if self
            .blocked_sections
            .write()
            .expect("RwLock for blocked_sections was poisoned")
            .remove(&section)
        {
            info!(section = %section, "unblocked config section after successful reload");
        }
    }

    /// Subscribe to config snapshot broadcasts.
    ///
    /// Returns a receiver that yields a new `ConfigSnapshot` each time
    /// a config section is successfully reloaded. Only the most recent
    /// snapshot is retained; lagging subscribers receive the latest one.
    pub fn subscribe_config_snapshots(&self) -> broadcast::Receiver<ConfigSnapshot> {
        self.snapshot_tx.subscribe()
    }

    /// Update the in-memory cache for a single section and broadcast
    /// the resulting snapshot to all snapshot subscribers.
    ///
    /// This is the single write-path for section updates that should
    /// trigger snapshot delivery. Callers that need to update the cache
    /// (e.g. `ConfigReloadManager::reload_section`) must go through
    /// this method instead of directly inserting into `sections`.
    pub fn update_section_cache(
        &self,
        section: ConfigSection,
        path: PathBuf,
        value: serde_json::Value,
    ) {
        self.unblock_section(section);
        let snapshot = {
            let mut sections = self
                .sections
                .write()
                .expect("RwLock for config sections was poisoned");
            sections.insert(section, value);
            ConfigSnapshot::new(sections.clone())
        };
        // Broadcast snapshot (ignore send errors — no active subscribers).
        let _ = self.snapshot_tx.send(snapshot);
        // Broadcast change event.
        self.notify_change(ConfigChangeEvent::Reloaded { section, path });
        info!(section = %section, "reloaded config section");
    }

    /// Get the loaded credentials provider.
    ///
    /// Returns `None` if `load()` has not been called yet.
    pub fn credentials(&self) -> Option<CredentialsProvider> {
        self.credentials_provider
            .read()
            .ok()
            .map(|guard| guard.clone())
    }

    /// Get the loaded session config provider.
    ///
    /// Returns `None` if `load()` has not been called yet.
    pub fn session_config_provider(&self) -> Option<Arc<dyn SessionConfigProvider>> {
        self.session_provider
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Subscribe to config change events.
    ///
    /// Returns a receiver that will receive all future config change events.
    /// Existing events published before subscription are not replayed.
    pub fn subscribe_config_changes(&self) -> tokio::sync::broadcast::Receiver<ConfigChangeEvent> {
        self.event_broadcaster.subscribe()
    }

    /// Publish a config change event to all active subscribers.
    pub fn notify_change(&self, event: ConfigChangeEvent) {
        self.event_broadcaster.send(event);
    }

    /// List metadata about all configuration files.
    ///
    /// Returns a vector of `ConfigInfo` for each section, including path,
    /// version (from JSON "version" field), and last modified timestamp.
    pub fn list_configs(&self) -> Vec<ConfigInfo> {
        let sections_list = [
            ConfigSection::Models,
            ConfigSection::Channels,
            ConfigSection::Gateway,
            ConfigSection::Plugins,
            ConfigSection::System,
            ConfigSection::Session,
            ConfigSection::Memory,
        ];

        let mut infos = Vec::new();

        for section in sections_list {
            let path = section.path(&self.config_dir);

            let metadata = match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let last_modified = metadata.modified().ok().map(DateTime::<Local>::from);

            let version = if let Ok(content) = fs::read_to_string(&path) {
                serde_json::from_str::<serde_json::Value>(&content)
                    .ok()
                    .and_then(|v| {
                        v.get("version")
                            .and_then(|vv| vv.as_str().map(str::to_string))
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };

            infos.push(ConfigInfo {
                path: path.to_string_lossy().to_string(),
                version,
                last_modified,
            });
        }

        infos
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
