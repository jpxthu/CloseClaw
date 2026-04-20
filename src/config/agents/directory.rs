//! Agent Directory Provider — loads per-agent config.json + permissions.json
//! from ~/.closeclaw/agents/<agent-name>/ directory structure.

use std::path::PathBuf;

use crate::agent::config::{AgentConfig as AgentDirConfig, AgentPermissions};
use crate::config::{ConfigError, ConfigProvider};

/// An agent loaded from a directory: its config.json and optional permissions.json.
#[derive(Debug, Clone)]
pub struct AgentDirectoryEntry {
    /// Agent ID (directory name).
    pub id: String,
    /// Parsed config.json.
    pub config: AgentDirConfig,
    /// Parsed permissions.json (None if absent).
    pub permissions: Option<AgentPermissions>,
}

/// AgentDirectoryProvider scans ~/.closeclaw/agents/ subdirectories and loads
/// each agent's config.json and permissions.json.
#[derive(Debug)]
pub struct AgentDirectoryProvider {
    agents_dir: PathBuf,
    entries: std::collections::HashMap<String, AgentDirectoryEntry>,
}

impl AgentDirectoryProvider {
    /// Create a new provider that scans the given agents base directory.
    pub fn new(agents_dir: PathBuf) -> Result<Self, ConfigError> {
        let mut provider = Self {
            agents_dir,
            entries: std::collections::HashMap::new(),
        };
        provider.reload()?;
        Ok(provider)
    }

    /// Reload all agent configs from disk.
    pub fn reload(&mut self) -> Result<(), ConfigError> {
        self.entries.clear();

        if !self.agents_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.agents_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = match path.file_name().and_then(|n| n.to_str()) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => continue,
            };

            let config_path = path.join("config.json");
            let config: AgentDirConfig = if config_path.exists() {
                let content = std::fs::read_to_string(&config_path)?;
                serde_json::from_str(&content).map_err(ConfigError::JsonError)?
            } else {
                continue;
            };

            let perm_path = path.join("permissions.json");
            let permissions: Option<AgentPermissions> = if perm_path.exists() {
                let content = std::fs::read_to_string(&perm_path)?;
                Some(serde_json::from_str(&content).map_err(
                    ConfigError::JsonError,
                )?)
            } else {
                None
            };

            self.entries.insert(
                id.clone(),
                AgentDirectoryEntry {
                    id,
                    config,
                    permissions,
                },
            );
        }

        self.validate()
    }

    /// Get an agent entry by id.
    pub fn get(&self, id: &str) -> Option<&AgentDirectoryEntry> {
        self.entries.get(id)
    }

    /// List all agent ids.
    pub fn agent_ids(&self) -> Vec<&String> {
        self.entries.keys().collect()
    }

    /// List all entries.
    pub fn entries(&self) -> &std::collections::HashMap<String, AgentDirectoryEntry> {
        &self.entries
    }

    /// Save an agent's config and optionally permissions to disk.
    pub fn save_agent(&self, entry: &AgentDirectoryEntry) -> Result<(), ConfigError> {
        let dir = self.agents_dir.join(&entry.id);
        std::fs::create_dir_all(&dir)?;

        let config_path = dir.join("config.json");
        let content =
            serde_json::to_string_pretty(&entry.config).map_err(ConfigError::JsonError)?;
        std::fs::write(&config_path, content)?;

        if let Some(ref perms) = entry.permissions {
            let perm_path = dir.join("permissions.json");
            let content = serde_json::to_string_pretty(perms).map_err(
                ConfigError::JsonError,
            )?;
            std::fs::write(&perm_path, content)?;
        }

        Ok(())
    }

    /// Remove an agent from the directory and in-memory entries.
    pub fn remove_agent(&mut self, id: &str) -> Result<(), ConfigError> {
        let dir = self.agents_dir.join(id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        self.entries.remove(id);
        Ok(())
    }
}

impl ConfigProvider for AgentDirectoryProvider {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "~/.closeclaw/agents/"
    }

    fn is_default(&self) -> bool {
        self.entries.is_empty()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        for (id, entry) in &self.entries {
            if entry.config.id.is_empty() {
                return Err(ConfigError::ValueError {
                    field: "id".to_string(),
                    message: format!("Agent '{}' has empty id", id),
                });
            }
        }
        Ok(())
    }
}
