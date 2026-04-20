//! Agent configuration module
//!
//! Provides AgentsConfigProvider for agents.json and AgentDirectoryProvider
//! for loading agent configurations from directories.

mod directory;
mod provider;
mod types;
mod validation;

pub use directory::{AgentDirectoryEntry, AgentDirectoryProvider};
pub use provider::AgentsConfigProvider;
pub use types::{AgentConfig, AgentsConfig};
pub use validation::validate_agents_config;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_config() {
        let json = r#"{
            "version": "1.0.0",
            "agents": [
                { "name": "orchestrator", "model": "gpt-4" },
                { "name": "coder", "model": "claude-3-opus", "parent": "orchestrator" }
            ]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        provider.validate().unwrap();
        assert!(provider.lookup().contains_key("orchestrator"));
    }

    #[test]
    fn test_missing_parent() {
        let json = r#"{
            "version": "1.0.0",
            "agents": [{ "name": "coder", "model": "claude-3-opus", "parent": "nonexistent" }]
        }"#;
        let provider = AgentsConfigProvider::from_json_str(json).unwrap();
        assert!(provider.validate().is_err());
    }
}
