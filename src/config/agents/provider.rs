//! Agents JSON ConfigProvider
//!
//! Loads and validates agents.json configuration.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::types::{AgentConfig, AgentsConfig};
use super::validation::validate_agents_config;
use crate::config::{ConfigError, ConfigProvider};

/// AgentsConfigProvider implements ConfigProvider for agents.json
#[derive(Debug, Clone)]
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
        Ok(Self { config, config_path })
    }

    /// Create a new provider from a string (useful for testing)
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: AgentsConfig = serde_json::from_str(content)?;
        Ok(Self {
            config,
            config_path: "memory".to_string(),
        })
    }

    /// Get the inner config
    pub fn inner(&self) -> &AgentsConfig {
        &self.config
    }

    /// Validate the config
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_agents_config(&self.config)
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
}

impl ConfigProvider for AgentsConfigProvider {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "agents.json"
    }

    fn is_default(&self) -> bool {
        self.config_path == "memory"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        validate_agents_config(&self.config)
    }
}
