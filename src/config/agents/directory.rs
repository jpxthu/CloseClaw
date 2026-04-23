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
                Some(serde_json::from_str(&content).map_err(ConfigError::JsonError)?)
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
            let content = serde_json::to_string_pretty(perms).map_err(ConfigError::JsonError)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{
        ActionPermission, AgentConfig as AgentDirConfig, AgentPermissions, PermissionLimits,
    };
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_config(id: &str, name: &str) -> AgentDirConfig {
        AgentDirConfig {
            id: id.to_string(),
            name: name.to_string(),
            parent_id: None,
            max_child_depth: 3,
            created_at: chrono::Utc::now(),
            state: crate::agent::config::AgentConfigState::Running,
            communication: crate::agent::communication::CommunicationConfig::default(),
            wait_timeout_secs: None,
            grace_period_secs: None,
        }
    }

    fn write_agent(
        dir: &std::path::Path,
        id: &str,
        config: &AgentDirConfig,
        perms: Option<&AgentPermissions>,
    ) {
        let agent_dir = dir.join(id);
        std::fs::create_dir_all(&agent_dir).unwrap();
        let config_json = serde_json::to_string_pretty(config).unwrap();
        std::fs::write(agent_dir.join("config.json"), config_json).unwrap();
        if let Some(p) = perms {
            let perm_json = serde_json::to_string_pretty(p).unwrap();
            std::fs::write(agent_dir.join("permissions.json"), perm_json).unwrap();
        }
    }

    #[test]
    fn test_new_empty_dir() {
        let dir = TempDir::new().unwrap();
        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert!(provider.agent_ids().is_empty());
        assert!(provider.is_default());
    }

    #[test]
    fn test_new_nonexistent_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent");
        let provider = AgentDirectoryProvider::new(path).unwrap();
        assert!(provider.agent_ids().is_empty());
    }

    #[test]
    fn test_load_single_agent() {
        let dir = TempDir::new().unwrap();
        let config = make_config("agent-1", "Agent One");
        write_agent(dir.path(), "agent-1", &config, None);

        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(provider.agent_ids().len(), 1);
        assert!(provider.get("agent-1").is_some());
        assert!(provider.get("agent-1").unwrap().permissions.is_none());
    }

    #[test]
    fn test_load_agent_with_permissions() {
        let dir = TempDir::new().unwrap();
        let config = make_config("agent-2", "Agent Two");
        let perms = AgentPermissions {
            agent_id: "agent-2".to_string(),
            permissions: HashMap::from([(
                "exec".to_string(),
                ActionPermission {
                    allowed: true,
                    limits: PermissionLimits::default(),
                },
            )]),
            inherited_from: None,
        };
        write_agent(dir.path(), "agent-2", &config, Some(&perms));

        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        let entry = provider.get("agent-2").unwrap();
        assert!(entry.permissions.is_some());
    }

    #[test]
    fn test_skip_dir_without_config() {
        let dir = TempDir::new().unwrap();
        let agent_dir = dir.path().join("empty-agent");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert!(provider.agent_ids().is_empty());
    }

    #[test]
    fn test_skip_files_in_agents_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "hello").unwrap();

        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert!(provider.agent_ids().is_empty());
    }

    #[test]
    fn test_version_and_config_path() {
        let dir = TempDir::new().unwrap();
        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(provider.version(), "1.0.0");
        assert_eq!(
            AgentDirectoryProvider::config_path(),
            "~/.closeclaw/agents/"
        );
    }

    #[test]
    fn test_validate_empty_id() {
        let dir = TempDir::new().unwrap();
        let config = AgentDirConfig {
            id: String::new(),
            ..make_config("x", "x")
        };
        // Manually insert entry with empty id
        write_agent(
            dir.path(),
            "bad-agent",
            &AgentDirConfig {
                id: String::new(),
                name: "Bad".to_string(),
                ..AgentDirConfig::default()
            },
            None,
        );

        let result = AgentDirectoryProvider::new(dir.path().to_path_buf());
        // config with empty id should fail validation
        assert!(result.is_err());
    }

    #[test]
    fn test_reload() {
        let dir = TempDir::new().unwrap();
        let config = make_config("a1", "Agent 1");
        write_agent(dir.path(), "a1", &config, None);

        let mut provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(provider.agent_ids().len(), 1);

        // Add another agent and reload
        let config2 = make_config("a2", "Agent 2");
        write_agent(dir.path(), "a2", &config2, None);
        provider.reload().unwrap();
        assert_eq!(provider.agent_ids().len(), 2);
    }

    #[test]
    fn test_save_agent() {
        let dir = TempDir::new().unwrap();
        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();

        let config = make_config("new-agent", "New");
        let entry = AgentDirectoryEntry {
            id: "new-agent".to_string(),
            config: config.clone(),
            permissions: None,
        };
        provider.save_agent(&entry).unwrap();

        // Verify written
        let written = std::fs::read_to_string(dir.path().join("new-agent/config.json")).unwrap();
        assert!(written.contains("new-agent"));
    }

    #[test]
    fn test_save_agent_with_permissions() {
        let dir = TempDir::new().unwrap();
        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();

        let config = make_config("perm-agent", "Perm");
        let perms = AgentPermissions {
            agent_id: "perm-agent".to_string(),
            permissions: HashMap::new(),
            inherited_from: None,
        };
        let entry = AgentDirectoryEntry {
            id: "perm-agent".to_string(),
            config,
            permissions: Some(perms),
        };
        provider.save_agent(&entry).unwrap();

        assert!(dir.path().join("perm-agent/permissions.json").exists());
    }

    #[test]
    fn test_remove_agent() {
        let dir = TempDir::new().unwrap();
        let config = make_config("rm-agent", "Remove");
        write_agent(dir.path(), "rm-agent", &config, None);

        let mut provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert!(provider.get("rm-agent").is_some());

        provider.remove_agent("rm-agent").unwrap();
        assert!(provider.get("rm-agent").is_none());
        assert!(!dir.path().join("rm-agent").exists());
    }

    #[test]
    fn test_entries() {
        let dir = TempDir::new().unwrap();
        let c1 = make_config("e1", "E1");
        let c2 = make_config("e2", "E2");
        write_agent(dir.path(), "e1", &c1, None);
        write_agent(dir.path(), "e2", &c2, None);

        let provider = AgentDirectoryProvider::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(provider.entries().len(), 2);
    }
}
