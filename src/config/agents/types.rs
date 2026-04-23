//! Agent configuration types
//!
//! Provides AgentConfig and AgentsConfig structs.

use serde::{Deserialize, Serialize};

/// Agent definition from JSON config
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    /// Agent identifier
    pub name: String,
    /// Model to use for this agent
    pub model: String,
    /// Optional human-readable persona description
    #[serde(default)]
    pub persona: String,
    /// Maximum iterations for this agent
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Timeout in minutes
    #[serde(default)]
    pub timeout_minutes: Option<u32>,
    /// Optional parent agent for delegation
    #[serde(default)]
    pub parent: Option<String>,
}

fn default_max_iterations() -> u32 {
    100
}

/// Wrapper for the entire agents.json file
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentsConfig {
    /// Config version
    pub version: String,
    /// List of agent definitions
    pub agents: Vec<AgentConfig>,
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

    #[test]
    fn test_agent_config_deserialize_full() {
        let json = r#"{
            "name": "test-agent",
            "model": "gpt-4",
            "persona": "A test agent",
            "max_iterations": 50,
            "timeout_minutes": 30,
            "parent": "parent-agent"
        }"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "test-agent");
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.persona, "A test agent");
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.timeout_minutes, Some(30));
        assert_eq!(config.parent, Some("parent-agent".to_string()));
    }

    #[test]
    fn test_agent_config_deserialize_minimal() {
        let json = r#"{
            "name": "minimal",
            "model": "claude-3"
        }"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "minimal");
        assert_eq!(config.model, "claude-3");
        assert_eq!(config.persona, "");
        assert_eq!(config.max_iterations, 100);
        assert_eq!(config.timeout_minutes, None);
        assert_eq!(config.parent, None);
    }

    #[test]
    fn test_agent_config_serialize_roundtrip() {
        let config = AgentConfig {
            name: "rt".to_string(),
            model: "m".to_string(),
            persona: "p".to_string(),
            max_iterations: 10,
            timeout_minutes: Some(5),
            parent: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, config.name);
        assert_eq!(parsed.max_iterations, config.max_iterations);
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
                {"name": "a1", "model": "m1"},
                {"name": "a2", "model": "m2", "max_iterations": 200}
            ]
        }"#;
        let config: AgentsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, "2.0");
        assert_eq!(config.agents.len(), 2);
        assert_eq!(config.agents[0].max_iterations, 100);
        assert_eq!(config.agents[1].max_iterations, 200);
    }

    #[test]
    fn test_agents_config_serialize_roundtrip() {
        let config = AgentsConfig {
            version: "1.0".to_string(),
            agents: vec![AgentConfig {
                name: "x".to_string(),
                model: "y".to_string(),
                persona: String::new(),
                max_iterations: 100,
                timeout_minutes: None,
                parent: Some("z".to_string()),
            }],
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.agents.len(), 1);
        assert_eq!(parsed.agents[0].parent.as_deref(), Some("z"));
    }
}
