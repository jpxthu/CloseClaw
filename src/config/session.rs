//! Session configuration module
//!
//! Provides per-agent per-role session configuration including idle timeout
//! and purge-after settings for the ArchiveSweeper and Daemon integrations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::providers::ConfigError;
use crate::session::persistence::AgentRole;

/// Default idle time in minutes before a session is considered idle
pub const DEFAULT_IDLE_MINUTES: i64 = 30;
/// Default purge time in minutes after which archived sessions are permanently deleted
pub const DEFAULT_PURGE_AFTER_MINUTES: i64 = 10080; // 7 days
/// Default sweeper interval in seconds
pub const DEFAULT_SWEEPER_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Per-agent per-role session configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerAgentSessionConfig {
    /// Idle timeout in minutes
    pub idle_minutes: i64,
    /// Purge-after timeout in minutes (0 = never purge)
    pub purge_after_minutes: i64,
}

impl Default for PerAgentSessionConfig {
    fn default() -> Self {
        Self {
            idle_minutes: DEFAULT_IDLE_MINUTES,
            purge_after_minutes: DEFAULT_PURGE_AFTER_MINUTES,
        }
    }
}

/// Session configuration container
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    /// Default per-role config for all agents
    pub defaults: BTreeMap<AgentRole, PerAgentSessionConfig>,
    /// Per-agent overrides (agent_id -> role -> config)
    #[serde(default)]
    pub agents: BTreeMap<String, BTreeMap<AgentRole, PerAgentSessionConfig>>,
}

/// Session configuration provider trait
pub trait SessionConfigProvider: Send + Sync {
    /// Get session config for a specific agent and role
    fn session_config_for(&self, agent_id: &str, role: AgentRole) -> PerAgentSessionConfig;

    /// Get sweeper interval in seconds
    fn sweeper_interval_secs(&self) -> u64;

    /// List all agent IDs that have per-agent overrides
    fn list_agents(&self) -> Vec<String>;
}
