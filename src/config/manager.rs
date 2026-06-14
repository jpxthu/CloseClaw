//! Config Manager — unified config management entry point
//!
//! Provides atomic write, backup integration, and unified access
//! to all JSON config files under the config/ directory.

use chrono::{DateTime, Local};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use thiserror::Error;
use tracing::{error, warn};
use uuid::Uuid;

use super::providers::{ConfigProvider, CredentialsProvider};
use crate::agent::config::AgentPermissions;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

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
pub fn write_atomically(path: &Path, content: &[u8]) -> io::Result<()> {
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
    Credentials,
}

impl ConfigSection {
    pub fn filename(&self) -> &'static str {
        match self {
            ConfigSection::Models => "models.json",
            ConfigSection::Channels => "channels.json",
            ConfigSection::Gateway => "gateway.json",
            ConfigSection::Plugins => "plugins.json",
            ConfigSection::System => "system.json",
            ConfigSection::Credentials => "credentials/",
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self, ConfigSection::Credentials)
    }

    pub fn dir_name(&self) -> &'static str {
        match self {
            ConfigSection::Credentials => "credentials",
            _ => panic!("dir_name() called on non-directory section: {:?}", self),
        }
    }

    pub(crate) fn path(&self, config_dir: &Path) -> PathBuf {
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
// Config helpers (module-level)
// ---------------------------------------------------------------------------

/// Read the "version" field from a JSON file.
/// Returns an empty string if the file is missing or unparseable.
fn read_json_version(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v.get("version")?.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Collect `ConfigInfo` entries from a directory of individual JSON files.
fn list_directory_configs(dir_path: &Path) -> Vec<ConfigInfo> {
    let mut infos = Vec::new();
    let Ok(entries) = fs::read_dir(dir_path) else {
        return infos;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let last_modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Local>::from);
        infos.push(ConfigInfo {
            path: path.to_string_lossy().to_string(),
            version: read_json_version(&path),
            last_modified,
        });
    }
    infos
}

/// Collect `ConfigInfo` for a single-file config section.
fn list_file_config(config_dir: &Path, section: &ConfigSection) -> Option<ConfigInfo> {
    let path = section.path(config_dir);
    let metadata = fs::metadata(&path).ok()?;
    let last_modified = metadata.modified().ok().map(DateTime::<Local>::from);
    Some(ConfigInfo {
        path: path.to_string_lossy().to_string(),
        version: read_json_version(&path),
        last_modified,
    })
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
    pub(crate) config_dir: PathBuf,
    /// Thread-safe backup manager.
    backup_manager: SafeBackupManager,
    /// In-memory cache of all loaded config sections.
    pub(crate) sections: RwLock<HashMap<ConfigSection, serde_json::Value>>,
    /// Loaded credentials provider (from config/credentials/ directory).
    credentials_provider: RwLock<CredentialsProvider>,
    /// Resolved agent configurations (loaded from two-level directories).
    pub(crate) agents: RwLock<HashMap<String, super::agents::ResolvedAgentConfig>>,
    /// Per-agent raw permissions (loaded from permissions.json).
    pub(crate) agent_permissions: RwLock<HashMap<String, AgentPermissions>>,
    /// Optional project root for loading project-level agents.json.
    pub(crate) repo_root: RwLock<Option<PathBuf>>,
}

impl ConfigManager {
    /// Create a new ConfigManager for the given config directory.
    ///
    /// The backup manager will store backups in `<config_dir>/.backups/`.
    pub fn new(config_dir: PathBuf) -> io::Result<Self> {
        let backup_dir = config_dir.join(".backups");
        let backup_manager =
            SafeBackupManager::new(super::backup::BackupManager::new(backup_dir, 10)?);
        Ok(Self {
            config_dir,
            backup_manager,
            sections: RwLock::new(HashMap::new()),
            credentials_provider: RwLock::new(CredentialsProvider::default()),
            agents: RwLock::new(HashMap::new()),
            agent_permissions: RwLock::new(HashMap::new()),
            repo_root: RwLock::new(None),
        })
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

        let mandatory_sections = [
            ConfigSection::Models,
            ConfigSection::Channels,
            ConfigSection::Gateway,
            ConfigSection::Plugins,
            ConfigSection::System,
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
                Err(_) => match self.try_rollback_and_retry(&path, section, &mut sections) {
                    Ok(()) => continue,
                    Err(e) => return Err(e),
                },
            };
            sections.insert(section, value);
        }

        // Load credentials from config/credentials/ directory.
        let creds_dir = self.config_dir.join(CredentialsProvider::config_path());
        let creds_provider = CredentialsProvider::load_from_dir(&creds_dir).unwrap_or_else(|e| {
            warn!(
                "failed to load credentials from '{}': {}",
                creds_dir.display(),
                e
            );
            CredentialsProvider::default()
        });
        *self.credentials_provider.write().expect("RwLock poisoned") = creds_provider.clone();
        if let Ok(json) = serde_json::to_value(&creds_provider) {
            sections.insert(ConfigSection::Credentials, json);
        }

        drop(sections);
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
        let backup_path = self.backup_manager.find_latest_backup(path).map_err(|_| {
            error!("配置文件 {} 损坏且无备份，daemon 无法启动", section);
            ConfigLoadError::ParseError {
                path: path.to_path_buf(),
                error: "no backup available".to_string(),
            }
        })?;

        if self.backup_manager.rollback(path).is_err() {
            error!("配置文件 {} 恢复后仍无法解析，daemon 无法启动", section);
            return Err(ConfigLoadError::ParseError {
                path: path.to_path_buf(),
                error: "rollback failed".to_string(),
            });
        }

        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                warn!("已用备份恢复 {}, 备份来源: {:?}", section, backup_path);
                sections.insert(section, val);
                return Ok(());
            }
        }

        error!("配置文件 {} 恢复后仍无法解析，daemon 无法启动", section);
        Err(ConfigLoadError::ParseError {
            path: path.to_path_buf(),
            error: "rollback succeeded but file still unparseable".to_string(),
        })
    }

    /// Update a configuration section.
    ///
    /// Flow: validate → backup → atomic write → update in-memory cache.
    pub fn update(
        &self,
        section: ConfigSection,
        new_value: serde_json::Value,
        validator: impl FnOnce(&serde_json::Value) -> Result<(), ConfigValidationError>,
    ) -> Result<(), ConfigWriteError> {
        if let Err(e) = validator(&new_value) {
            return Err(ConfigWriteError::ValidationFailed(
                section.to_string(),
                e.message.clone(),
            ));
        }

        let path = section.path(&self.config_dir);
        if path.exists() {
            let content = fs::read(&path).map_err(|e| ConfigWriteError::BackupFailed {
                path: path.clone(),
                error: e.to_string(),
            })?;
            self.backup_manager
                .backup_with_content(&path, &content)
                .map_err(|e: io::Error| ConfigWriteError::BackupFailed {
                    path: path.clone(),
                    error: e.to_string(),
                })?;
        }

        let bytes =
            serde_json::to_vec_pretty(&new_value).map_err(|e| ConfigWriteError::WriteFailed {
                path: path.clone(),
                error: e.to_string(),
            })?;
        write_atomically(&path, &bytes).map_err(|e| ConfigWriteError::WriteFailed {
            path,
            error: e.to_string(),
        })?;

        self.sections
            .write()
            .expect("RwLock for config sections was poisoned")
            .insert(section, new_value);
        Ok(())
    }

    /// Get a read-only clone of a configuration section's JSON value.
    ///
    /// Returns `None` if the section has not been loaded.
    pub fn section(&self, section: ConfigSection) -> Option<serde_json::Value> {
        self.sections
            .read()
            .expect("RwLock for config sections was poisoned")
            .get(&section)
            .cloned()
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

    /// List metadata about all configuration files.
    pub fn list_configs(&self) -> Vec<ConfigInfo> {
        let sections = [
            ConfigSection::Models,
            ConfigSection::Channels,
            ConfigSection::Gateway,
            ConfigSection::Plugins,
            ConfigSection::System,
            ConfigSection::Credentials,
        ];
        sections
            .iter()
            .flat_map(|s| {
                if s.is_directory() {
                    list_directory_configs(&self.config_dir.join(s.dir_name()))
                } else {
                    list_file_config(&self.config_dir, s).into_iter().collect()
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Module exports
// ---------------------------------------------------------------------------

pub use super::backup::SafeBackupManager;

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
