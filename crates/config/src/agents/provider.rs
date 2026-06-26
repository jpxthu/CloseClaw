//! Agents JSON ConfigProvider
//!
//! Loads and validates agents.json (registration list of agent IDs).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::validation::validate_agents_config;
use super::AgentsConfig;
use crate::{ConfigError, ConfigProvider};

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

    /// Get the inner config
    pub fn inner(&self) -> &AgentsConfig {
        &self.config
    }

    /// Validate the config
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_agents_config(&self.config)
    }

    /// Check whether the given agent ID is registered.
    pub fn get(&self, id: &str) -> Option<&str> {
        self.config
            .agents
            .iter()
            .find(|a| a.as_str() == id)
            .map(String::as_str)
    }

    /// Get all registered agent IDs.
    pub fn agents(&self) -> &[String] {
        &self.config.agents
    }

    /// Build a lookup map of agent ID -> ID (useful for membership tests).
    pub fn lookup(&self) -> HashMap<&str, &str> {
        self.config
            .agents
            .iter()
            .map(|id| (id.as_str(), id.as_str()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let provider = AgentsConfigProvider::default();
        assert!(provider.is_default());
        assert!(provider.agents().is_empty());
    }

    #[test]
    fn test_version() {
        let provider = AgentsConfigProvider::default();
        assert_eq!(provider.version(), "1.0.0");
    }

    #[test]
    fn test_config_path() {
        assert_eq!(AgentsConfigProvider::config_path(), "agents.json");
    }

    #[test]
    fn test_from_json_str_valid() {
        let json = r#"{
            "agents": ["agent-a", "agent-b"]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.is_default()); // from_json_str uses "memory" path
        assert_eq!(provider.agents().len(), 2);
    }

    #[test]
    fn test_from_json_str_invalid() {
        let json = "not json";
        let result = AgentsConfigProvider::from_json_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_inner() {
        let json = r#"{"agents":["x"]}"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert_eq!(provider.inner().agents, vec!["x".to_string()]);
    }

    #[test]
    fn test_get_found() {
        let json = r#"{"agents":["alpha","beta"]}"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert_eq!(provider.get("alpha"), Some("alpha"));
        assert_eq!(provider.get("beta"), Some("beta"));
    }

    #[test]
    fn test_get_not_found() {
        let json = r#"{"agents":["alpha"]}"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.get("nonexistent").is_none());
    }

    #[test]
    fn test_lookup() {
        let json = r#"{"agents":["a","b"]}"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        let map = provider.lookup();
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("a"));
        assert!(map.contains_key("b"));
    }

    #[test]
    fn test_validate_valid() {
        let json = r#"{"agents":["agent1","agent2"]}"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_duplicate() {
        let json = r#"{"agents":["agent1","agent1"]}"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_empty_id() {
        let json = r#"{"agents":[""]}"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.validate().is_err());
    }

    #[test]
    fn test_new_from_file_missing() {
        let result = AgentsConfigProvider::new("/nonexistent/path/agents.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_new_from_temp_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("agents.json");
        let json = r#"{"agents":["test"]}"#;
        std::fs::write(&path, json).unwrap();
        let provider = AgentsConfigProvider::new(&path).unwrap();
        assert_eq!(provider.agents().len(), 1);
        assert!(!provider.is_default());
    }
}
