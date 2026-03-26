//! Agents JSON ConfigProvider
//!
//! Loads and validates agents.json configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::{ConfigError, ConfigProvider};

/// Agent definition from JSON config
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    /// Agent identifier
    pub name: String,
    /// Model to use for this agent
    pub model: String,
    /// Optional parent agent name for inheritance
    #[serde(default)]
    pub parent: Option<String>,
    /// Agent persona/description
    #[serde(default)]
    pub persona: String,
    /// Maximum iteration limit for this agent
    #[serde(default)]
    pub max_iterations: Option<u32>,
    /// Timeout in minutes for agent tasks
    #[serde(default)]
    pub timeout_minutes: Option<u32>,
}

/// Wrapper for the entire agents.json file
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentsConfig {
    /// Config version for tracking
    pub version: String,
    /// List of agent definitions
    pub agents: Vec<AgentConfig>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
            agents: Vec::new(),
        }
    }
}

/// AgentsConfigProvider implements ConfigProvider for agents.json
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentsConfigProvider {
    config: AgentsConfig,
    config_path: String,
}

impl Default for AgentsConfigProvider {
    fn default() -> Self {
        Self {
            config: AgentsConfig::default(),
            config_path: "memory".to_string(),
        }
    }
}

impl AgentsConfigProvider {
    /// Create a new provider from file path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let config_path = path.as_ref().display().to_string();
        let content = fs::read_to_string(path)?;
        let config: AgentsConfig = serde_json::from_str(&content)?;
        Ok(Self {
            config,
            config_path,
        })
    }

    /// Create a new provider from a string (useful for testing)
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: AgentsConfig = serde_json::from_str(content)?;
        Ok(Self {
            config,
            config_path: "memory".to_string(),
        })
    }

    /// Get an agent config by name
    pub fn get(&self, name: &str) -> Option<&AgentConfig> {
        self.config.agents.iter().find(|a| a.name == name)
    }

    /// Get all agents
    pub fn agents(&self) -> &[AgentConfig] {
        &self.config.agents
    }

    /// Build a lookup map of agent name -> AgentConfig
    pub fn lookup(&self) -> HashMap<&str, &AgentConfig> {
        self.config
            .agents
            .iter()
            .map(|a| (a.name.as_str(), a))
            .collect()
    }

    /// Reload config from disk
    pub fn reload(&mut self) -> Result<(), ConfigError> {
        let content = fs::read_to_string(&self.config_path)?;
        let config: AgentsConfig = serde_json::from_str(&content)?;
        config.validate()?;
        self.config = config;
        Ok(())
    }

    /// Get the raw config
    pub fn inner(&self) -> &AgentsConfig {
        &self.config
    }

    /// Get a mutable reference to the raw config
    pub fn inner_mut(&mut self) -> &mut AgentsConfig {
        &mut self.config
    }
}

impl ConfigProvider for AgentsConfigProvider {
    fn version(&self) -> &'static str {
        // Note: the trait requires &'static str, but our config has owned String.
        // For AgentsConfigProvider, we return a static string constant.
        // For dynamic versions, the ConfigProvider trait should return String.
        static VERSION: &str = "1.0.0";
        VERSION
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "agents.json"
    }

    fn is_default(&self) -> bool {
        self.config_path == "memory" || self.config.agents.is_empty()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        self.config.validate()?;
        // Build lookup to validate parent references
        let lookup = self.lookup();
        for agent in &self.config.agents {
            if let Some(ref parent) = agent.parent {
                if !lookup.contains_key(parent.as_str()) {
                    return Err(ConfigError::ValueError {
                        field: "parent".to_string(),
                        message: format!(
                            "Agent '{}' references unknown parent '{}'",
                            agent.name, parent
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

impl AgentsConfig {
    /// Validate the agents config schema
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version.is_empty() {
            return Err(ConfigError::ValueError {
                field: "version".to_string(),
                message: "version cannot be empty".to_string(),
            });
        }

        let mut names = std::collections::HashSet::new();
        for agent in &self.agents {
            if agent.name.is_empty() {
                return Err(ConfigError::ValueError {
                    field: "name".to_string(),
                    message: "Agent name cannot be empty".to_string(),
                });
            }
            if agent.model.is_empty() {
                return Err(ConfigError::ValueError {
                    field: "model".to_string(),
                    message: format!("Agent '{}' has empty model", agent.name),
                });
            }
            if !names.insert(&agent.name) {
                return Err(ConfigError::ValueError {
                    field: "name".to_string(),
                    message: format!("Duplicate agent name '{}'", agent.name),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_config() {
        let json = r#"{
            "version": "1.0.0",
            "agents": [
                {
                    "name": "orchestrator",
                    "model": "gpt-4",
                    "persona": "Master orchestrator",
                    "max_iterations": 100,
                    "timeout_minutes": 60
                },
                {
                    "name": "coder",
                    "model": "claude-3-opus",
                    "parent": "orchestrator"
                }
            ]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        provider.validate().unwrap();

        let lookup = provider.lookup();
        assert!(lookup.contains_key("orchestrator"));
        assert!(lookup.contains_key("coder"));

        let coder = provider.get("coder").unwrap();
        assert_eq!(coder.parent.as_deref(), Some("orchestrator"));
    }

    #[test]
    fn test_missing_parent() {
        let json = r#"{
            "version": "1.0.0",
            "agents": [
                {
                    "name": "coder",
                    "model": "claude-3-opus",
                    "parent": "nonexistent"
                }
            ]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        let err = provider.validate().unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_duplicate_agent() {
        let json = r#"{
            "version": "1.0.0",
            "agents": [
                { "name": "agent1", "model": "gpt-4" },
                { "name": "agent1", "model": "claude-3-opus" }
            ]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        let err = provider.validate().unwrap_err();
        assert!(err.to_string().contains("Duplicate"));
    }
}

// ---------------------------------------------------------------------------
// Agent Directory Provider — loads per-agent config.json + permissions.json
// from ~/.closeclaw/agents/<agent-name>/ directory structure.
// ---------------------------------------------------------------------------

use crate::agent::config::{AgentConfig as AgentDirConfig, AgentPermissions};
use std::path::PathBuf;

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
    /// Map from agent id -> entry.
    entries: HashMap<String, AgentDirectoryEntry>,
}

impl AgentDirectoryProvider {
    /// Create a new provider that scans the given agents base directory.
    pub fn new(agents_dir: PathBuf) -> Result<Self, ConfigError> {
        let mut provider = Self {
            agents_dir,
            entries: HashMap::new(),
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

        for entry in fs::read_dir(&self.agents_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = match path.file_name().and_then(|n| n.to_str()) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => continue, // Skip directories with non-UTF8 or empty names
            };

            // Load config.json
            let config_path = path.join("config.json");
            let config: AgentDirConfig = if config_path.exists() {
                let content = fs::read_to_string(&config_path)?;
                serde_json::from_str(&content).map_err(ConfigError::JsonError)?
            } else {
                continue; // Skip dirs without config.json
            };

            // Load permissions.json (optional)
            let perm_path = path.join("permissions.json");
            let permissions: Option<AgentPermissions> = if perm_path.exists() {
                let content = fs::read_to_string(&perm_path)?;
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
    pub fn entries(&self) -> &HashMap<String, AgentDirectoryEntry> {
        &self.entries
    }

    /// Save an agent's config and optionally permissions to disk.
    /// Creates the directory if it doesn't exist.
    pub fn save_agent(&self, entry: &AgentDirectoryEntry) -> Result<(), ConfigError> {
        let dir = self.agents_dir.join(&entry.id);
        fs::create_dir_all(&dir)?;

        let config_path = dir.join("config.json");
        let content =
            serde_json::to_string_pretty(&entry.config).map_err(ConfigError::JsonError)?;
        fs::write(&config_path, content)?;

        if let Some(ref perms) = entry.permissions {
            let perm_path = dir.join("permissions.json");
            let content = serde_json::to_string_pretty(perms).map_err(ConfigError::JsonError)?;
            fs::write(&perm_path, content)?;
        }

        Ok(())
    }

    /// Remove an agent from the directory and in-memory entries.
    pub fn remove_agent(&mut self, id: &str) -> Result<(), ConfigError> {
        let dir = self.agents_dir.join(id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
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
mod agent_dir_tests {
    use super::*;

    #[test]
    fn test_agent_directory_load_and_save() {
        use chrono::Utc;
        use std::fs;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let agents_dir = temp.path().to_path_buf();

        // Create an agent directory manually
        let agent_dir = agents_dir.join("test-agent");
        fs::create_dir_all(&agent_dir).unwrap();

        let config = AgentDirConfig {
            id: "test-agent".to_string(),
            name: "Test Agent".to_string(),
            parent_id: None,
            max_child_depth: 2,
            created_at: Utc::now(),
            state: crate::agent::config::AgentConfigState::Running,
            communication: Default::default(),
        };

        let config_path = agent_dir.join("config.json");
        fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let provider = AgentDirectoryProvider::new(agents_dir).unwrap();
        let entry = provider.get("test-agent").unwrap();
        assert_eq!(entry.config.name, "Test Agent");
        assert_eq!(entry.config.max_child_depth, 2);
    }

    #[test]
    fn test_save_and_remove_agent() {
        use chrono::Utc;
        use std::fs;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let agents_dir = temp.path().to_path_buf();
        fs::create_dir_all(&agents_dir).unwrap();

        let provider = AgentDirectoryProvider::new(agents_dir.clone()).unwrap();

        let entry = AgentDirectoryEntry {
            id: "new-agent".to_string(),
            config: AgentDirConfig {
                id: "new-agent".to_string(),
                name: "New Agent".to_string(),
                parent_id: None,
                max_child_depth: 3,
                created_at: Utc::now(),
                state: crate::agent::config::AgentConfigState::Running,
                communication: Default::default(),
            },
            permissions: None,
        };

        provider.save_agent(&entry).unwrap();

        // Reload and verify
        let mut provider2 = AgentDirectoryProvider::new(agents_dir).unwrap();
        let loaded = provider2.get("new-agent").unwrap();
        assert_eq!(loaded.config.name, "New Agent");

        // Remove
        provider2.remove_agent("new-agent").unwrap();
        assert!(provider2.get("new-agent").is_none());
    }

    #[test]
    fn test_reload_skips_non_utf8_directory_names() {
        use chrono::Utc;
        use std::fs;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let agents_dir = temp.path().to_path_buf();

        // Create a valid agent directory
        let valid_dir = agents_dir.join("valid-agent");
        fs::create_dir_all(&valid_dir).unwrap();
        let config = AgentDirConfig {
            id: "valid-agent".to_string(),
            name: "Valid Agent".to_string(),
            parent_id: None,
            max_child_depth: 2,
            created_at: Utc::now(),
            state: crate::agent::config::AgentConfigState::Running,
            communication: Default::default(),
        };
        fs::write(
            valid_dir.join("config.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        // Create an agent directory with invalid UTF-8 name (os_str without valid utf8)
        // We simulate this by creating a directory with a name that would fail to_str()
        // On Linux we can use OsString::from_vec to create non-UTF8 bytes
        use std::os::unix::ffi::OsStringExt;
        let non_utf8_name: std::ffi::OsString =
            std::ffi::OsString::from_vec(vec![0x80, 0x81, 0x82]);
        let invalid_dir = agents_dir.join(&non_utf8_name);
        fs::create_dir_all(&invalid_dir).unwrap();

        // reload() should skip the non-UTF8 directory and succeed
        let mut provider = AgentDirectoryProvider::new(agents_dir.clone()).unwrap();
        let result = provider.reload();
        assert!(
            result.is_ok(),
            "reload should skip non-UTF8 dirs and succeed"
        );
        assert!(
            provider.get("valid-agent").is_some(),
            "valid agent should still be loaded"
        );
    }
}
