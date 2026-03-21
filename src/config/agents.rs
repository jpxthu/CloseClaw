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
    /// Parse from a JSON string (used by reload manager).
    pub fn from_json_str(s: &str) -> Result<Self, ConfigError> {
        serde_json::from_str(s).map_err(ConfigError::JsonError)
    }
    /// Create a new provider from file path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let config_path = path.as_ref().display().to_string();
        let content = fs::read_to_string(path)?;
        let config: AgentsConfig = serde_json::from_str(&content)?;
        Ok(Self { config, config_path })
    }

    /// Create a new provider from a string (useful for testing)
    pub fn from_str(content: &str) -> Result<Self, ConfigError> {
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
                    message: format!(
                        "Agent '{}' has empty model",
                        agent.name
                    ),
                });
            }
            if !names.insert(&agent.name) {
                return Err(ConfigError::ValueError {
                    field: "name".to_string(),
                    message: format!(
                        "Duplicate agent name '{}'",
                        agent.name
                    ),
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
        let provider = AgentsConfigProvider::from_str(json).unwrap();
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
        let provider = AgentsConfigProvider::from_str(json).unwrap();
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
        let provider = AgentsConfigProvider::from_str(json).unwrap();
        let err = provider.validate().unwrap_err();
        assert!(err.to_string().contains("Duplicate"));
    }
}
