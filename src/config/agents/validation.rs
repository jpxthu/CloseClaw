//! Agent configuration validation
//!
//! Provides validation logic for AgentsConfig.

use super::AgentsConfig;
use crate::config::ConfigError;

/// Validation result type
pub type ValidationResult = Result<(), ConfigError>;

/// Validate agent configuration
pub fn validate_agents_config(config: &AgentsConfig) -> ValidationResult {
    let mut names = std::collections::HashSet::new();
    for agent in &config.agents {
        if agent.name.is_empty() {
            return Err(ConfigError::ValueError {
                field: "name".to_string(),
                message: "Agent name cannot be empty".to_string(),
            });
        }
        if agent.model.as_ref().map_or(true, |m| m.is_empty()) {
            return Err(ConfigError::ValueError {
                field: "model".to_string(),
                message: format!("Agent '{}' has empty model", agent.name),
            });
        }
        if !names.insert(agent.name.clone()) {
            return Err(ConfigError::ValueError {
                field: "name".to_string(),
                message: format!("Duplicate agent name '{}'", agent.name),
            });
        }
    }

    // Validate parent references
    for agent in &config.agents {
        if let Some(ref parent_id) = agent.parent_id {
            if !names.contains(parent_id) {
                return Err(ConfigError::ValueError {
                    field: "parent".to_string(),
                    message: format!(
                        "Agent '{}' references non-existent parent '{}'",
                        agent.name, parent_id
                    ),
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::AgentConfig;

    fn make_agent(name: &str, model: &str) -> AgentConfig {
        AgentConfig {
            id: name.to_string(),
            name: name.to_string(),
            model: Some(model.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_valid_config() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![make_agent("orchestrator", "gpt-4")],
        };
        validate_agents_config(&config).unwrap();
    }

    #[test]
    fn test_empty_name() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![AgentConfig {
                id: String::new(),
                name: String::new(),
                model: Some("gpt-4".to_string()),
                ..Default::default()
            }],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_empty_model() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![AgentConfig {
                id: "agent1".to_string(),
                name: "agent1".to_string(),
                model: None,
                ..Default::default()
            }],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("empty model"));
    }

    #[test]
    fn test_duplicate_agent() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![
                make_agent("agent1", "gpt-4"),
                make_agent("agent1", "claude-3-opus"),
            ],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("Duplicate"));
    }

    #[test]
    fn test_parent_not_found() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![AgentConfig {
                id: "coder".to_string(),
                name: "coder".to_string(),
                model: Some("claude-3-opus".to_string()),
                parent_id: Some("nonexistent".to_string()),
                ..Default::default()
            }],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("parent"));
        assert!(err.to_string().contains("nonexistent"));
    }
}
