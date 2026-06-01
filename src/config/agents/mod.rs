//! Agent configuration module
//!
//! Provides AgentsConfigProvider for agents.json (registration list of agent IDs)
//! and AgentDirectoryProvider for loading agent configurations from directories.

mod directory;
mod provider;
mod resolved;
mod types;
mod validation;

pub use crate::agent::config::AgentConfig;
pub use directory::AgentDirectoryProvider;
pub use provider::AgentsConfigProvider;
pub use resolved::{ConfigSource, ResolvedAgentConfig};
pub use types::AgentsConfig;
pub use validation::validate_agents_config;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_config() {
        let json = r#"{
            "agents": ["orchestrator", "coder", "tester"]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        provider.validate().unwrap();
        assert!(provider.lookup().contains_key("orchestrator"));
        assert!(provider.lookup().contains_key("coder"));
        assert!(provider.lookup().contains_key("tester"));
    }

    #[test]
    fn test_duplicate_id_rejected() {
        let json = r#"{
            "agents": ["agent", "agent"]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.validate().is_err());
    }

    #[test]
    fn test_empty_id_rejected() {
        let json = r#"{
            "agents": [""]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.validate().is_err());
    }
}
