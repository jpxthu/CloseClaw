//! Agent configuration types
//!
//! Provides AgentsConfig struct.
//! AgentConfig is defined in `crate::agent::config`.

use serde::{Deserialize, Serialize};

/// Wrapper for the entire agents.json file
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentsConfig {
    /// Config version
    pub version: String,
    /// List of agent definitions
    pub agents: Vec<crate::agent::config::AgentConfig>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
            agents: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{AgentConfig, AgentConfigState};

    fn make_agent(name: &str, model: &str) -> AgentConfig {
        AgentConfig {
            id: name.to_string(),
            name: name.to_string(),
            model: Some(model.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_agent_config_deserialize_full() {
        let json = r#"{
            "id": "test-id",
            "name": "test-agent",
            "model": "gpt-4",
            "created_at": "2026-01-01T00:00:00Z"
        }"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.id, "test-id");
        assert_eq!(config.name, "test-agent");
        assert_eq!(config.model.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn test_agent_config_deserialize_minimal() {
        let json = r#"{
            "id": "min-id",
            "name": "minimal",
            "created_at": "2026-01-01T00:00:00Z"
        }"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.id, "min-id");
        assert_eq!(config.name, "minimal");
        assert_eq!(config.model, None);
    }

    #[test]
    fn test_agent_config_serialize_roundtrip() {
        let config = AgentConfig {
            id: "rt".to_string(),
            name: "rt".to_string(),
            model: Some("m".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, config.id);
        assert_eq!(parsed.name, config.name);
    }

    #[test]
    fn test_agents_config_default() {
        let config = AgentsConfig::default();
        assert_eq!(config.version, "1.0.0");
        assert!(config.agents.is_empty());
    }

    #[test]
    fn test_agents_config_deserialize() {
        let json = r#"{
            "version": "2.0",
            "agents": [
                {"id": "a1", "name": "a1", "model": "m1", "created_at": "2026-01-01T00:00:00Z"},
                {"id": "a2", "name": "a2", "model": "m2", "created_at": "2026-01-01T00:00:00Z"}
            ]
        }"#;
        let config: AgentsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, "2.0");
        assert_eq!(config.agents.len(), 2);
        assert_eq!(config.agents[0].model.as_deref(), Some("m1"));
        assert_eq!(config.agents[1].model.as_deref(), Some("m2"));
    }

    #[test]
    fn test_agents_config_serialize_roundtrip() {
        let config = AgentsConfig {
            version: "1.0".to_string(),
            agents: vec![make_agent("x", "y")],
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.agents.len(), 1);
        assert_eq!(parsed.agents[0].name, "x");
    }
}
