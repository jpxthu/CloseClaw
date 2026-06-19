//! Agent configuration types
//!
//! Provides AgentsConfig struct, which only stores a list of registered
//! agent IDs (parsed from the JSONC `agents.json` registration list).
//! Full agent configuration is loaded per-agent from its own directory
//! (see `crate::agent::config::AgentConfig` and `AgentDirectoryProvider`).

use serde::{Deserialize, Serialize};

/// Agent registration list from agents.json (JSONC format).
/// Only contains registered agent IDs; commented-out IDs are excluded.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AgentsConfig {
    /// Registered agent ID list
    pub agents: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agents_config_default() {
        let config = AgentsConfig::default();
        assert!(config.agents.is_empty());
    }

    #[test]
    fn test_agents_config_deserialize() {
        let json = r#"{
            "agents": ["a1", "a2", "orchestrator"]
        }"#;
        let config: AgentsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents.len(), 3);
        assert_eq!(config.agents[0], "a1");
        assert_eq!(config.agents[1], "a2");
        assert_eq!(config.agents[2], "orchestrator");
    }

    #[test]
    fn test_agents_config_deserialize_empty() {
        let json = r#"{"agents": []}"#;
        let config: AgentsConfig = serde_json::from_str(json).unwrap();
        assert!(config.agents.is_empty());
    }

    #[test]
    fn test_agents_config_serialize_roundtrip() {
        let config = AgentsConfig {
            agents: vec!["x".to_string(), "y".to_string()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.agents.len(), 2);
        assert_eq!(parsed.agents[0], "x");
        assert_eq!(parsed.agents[1], "y");
    }
}
