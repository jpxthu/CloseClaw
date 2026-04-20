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
