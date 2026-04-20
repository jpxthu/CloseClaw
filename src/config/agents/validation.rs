//! Agent configuration validation
//!
//! Provides validation logic for AgentsConfig.

use super::types::{AgentConfig, AgentsConfig};
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

    // Validate parent references
    for agent in &config.agents {
        if let Some(ref parent) = agent.parent {
            if !names.contains(parent) {
                return Err(ConfigError::ValueError {
                    field: "parent".to_string(),
                    message: format!(
                        "Agent '{}' references non-existent parent '{}'",
                        agent.name, parent
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

    #[test]
    fn test_valid_config() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![
                crate::config::agents::types::AgentConfig {
                    name: "orchestrator".to_string(),
                    model: "gpt-4".to_string(),
                    persona: "Master orchestrator".to_string(),
                    max_iterations: 100,
                    timeout_minutes: Some(60),
                    parent: None,
                },
            ],
        };
        validate_agents_config(&config).unwrap();
    }

    #[test]
    fn test_empty_name() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![crate::config::agents::types::AgentConfig {
                name: "".to_string(),
                model: "gpt-4".to_string(),
                persona: "".to_string(),
                max_iterations: 100,
                timeout_minutes: None,
                parent: None,
            }],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_empty_model() {
        let config = AgentsConfig {
            version: "1.0.0".to_string(),
            agents: vec![crate::config::agents::types::AgentConfig {
                name: "agent1".to_string(),
                model: "".to_string(),
                persona: "".to_string(),
                max_iterations: 100,
                timeout_minutes: None,
                parent: None,
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
                crate::config::agents::types::AgentConfig {
                    name: "agent1".to_string(),
                    model: "gpt-4".to_string(),
                    persona: "".to_string(),
                    max_iterations: 100,
                    timeout_minutes: None,
                    parent: None,
                },
                crate::config::agents::types::AgentConfig {
                    name: "agent1".to_string(),
                    model: "claude-3-opus".to_string(),
                    persona: "".to_string(),
                    max_iterations: 100,
                    timeout_minutes: None,
                    parent: None,
                },
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
                name: "coder".to_string(),
                model: "claude-3-opus".to_string(),
                persona: "Coder".to_string(),
                max_iterations: 100,
                timeout_minutes: Some(60),
                parent: Some("nonexistent".to_string()),
            }],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("parent"));
        assert!(err.to_string().contains("nonexistent"));
    }
}
