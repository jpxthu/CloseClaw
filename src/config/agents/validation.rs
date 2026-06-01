//! Agent configuration validation
//!
//! Provides validation logic for AgentsConfig.

use super::AgentsConfig;
use crate::config::ConfigError;

/// Validation result type
pub type ValidationResult = Result<(), ConfigError>;

/// Validate agent registration list.
///
/// Verifies that:
/// - Every ID is non-empty
/// - There are no duplicate IDs
pub fn validate_agents_config(config: &AgentsConfig) -> ValidationResult {
    let mut seen = std::collections::HashSet::new();
    for id in &config.agents {
        if id.is_empty() {
            return Err(ConfigError::ValueError {
                field: "id".to_string(),
                message: "Agent ID cannot be empty".to_string(),
            });
        }
        if !seen.insert(id.clone()) {
            return Err(ConfigError::ValueError {
                field: "id".to_string(),
                message: format!("Duplicate agent ID '{}'", id),
            });
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
            agents: vec!["orchestrator".to_string(), "coder".to_string()],
        };
        validate_agents_config(&config).unwrap();
    }

    #[test]
    fn test_empty_id() {
        let config = AgentsConfig {
            agents: vec!["".to_string()],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_duplicate_agent_id() {
        let config = AgentsConfig {
            agents: vec!["agent1".to_string(), "agent1".to_string()],
        };
        let err = validate_agents_config(&config).unwrap_err();
        assert!(err.to_string().contains("Duplicate"));
    }

    #[test]
    fn test_empty_list_is_valid() {
        let config = AgentsConfig::default();
        assert!(validate_agents_config(&config).is_ok());
    }
}
