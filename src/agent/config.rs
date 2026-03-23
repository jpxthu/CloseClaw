//! Agent Configuration - config.json and permissions.json structures for per-agent config files.
//!
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Communication whitelist for an agent.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CommunicationConfig {
    /// Agent IDs this agent is allowed to send messages to.
    #[serde(default)]
    pub outbound: Vec<String>,
    /// Agent IDs this agent is allowed to receive messages from.
    #[serde(default)]
    pub inbound: Vec<String>,
}

impl CommunicationConfig {
    /// Returns the default communication config: only parent is allowed.
    pub fn default_with_parent(parent_id: Option<&str>) -> Self {
        let parent_list: Vec<String> = parent_id.map(String::from).into_iter().collect();
        Self {
            outbound: parent_list.clone(),
            inbound: parent_list,
        }
    }
}

/// Agent's own configuration (stored as config.json in the agent's directory).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    /// Unique identifier for this agent.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Parent agent ID (if this agent was spawned by another).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Maximum depth of child hierarchy this agent can create.
    #[serde(default = "default_max_child_depth")]
    pub max_child_depth: u32,
    /// When this agent was created.
    pub created_at: DateTime<Utc>,
    /// Current operational state.
    #[serde(default)]
    pub state: AgentConfigState,
    /// Communication whitelist configuration.
    #[serde(default)]
    pub communication: CommunicationConfig,
}

fn default_max_child_depth() -> u32 {
    3
}

/// Operational state of an agent (distinct from runtime `AgentState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentConfigState {
    /// Active and processing.
    #[default]
    Running,
    /// Suspended/paused.
    Suspended,
    /// Stopped.
    Stopped,
}

impl AgentConfig {
    /// Load config from a JSON file at the given path.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Save config to a JSON file at the given path.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, content)
    }
}

/// Permission limits for a single action category.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PermissionLimits {
    /// Allowed commands (for exec).
    #[serde(default)]
    pub commands: Vec<String>,
    /// Allowed paths (for file_read/file_write).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Timeout limit in milliseconds (for exec).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Permissions for a single action category.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ActionPermission {
    /// Whether this action is allowed.
    #[serde(default)]
    pub allowed: bool,
    /// Optional limits when allowed.
    #[serde(default)]
    pub limits: PermissionLimits,
}

/// Full permissions configuration for an agent (stored as permissions.json).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AgentPermissions {
    /// Agent identifier these permissions apply to.
    pub agent_id: String,
    /// Permission rules by action category.
    #[serde(default)]
    pub permissions: HashMap<String, ActionPermission>,
    /// ID of the agent from which these permissions are inherited.
    #[serde(default)]
    pub inherited_from: Option<String>,
}

impl AgentPermissions {
    /// Load permissions from a JSON file.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Save permissions to a JSON file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, content)
    }

    /// Check if a specific action is permitted.
    pub fn is_allowed(&self, action: &str) -> bool {
        self.permissions
            .get(action)
            .map(|p| p.allowed)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_agent_config_save_load() {
        let temp = TempDir::new().unwrap();
        let config = AgentConfig {
            id: "test-id".to_string(),
            name: "Test Agent".to_string(),
            parent_id: Some("parent-id".to_string()),
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: CommunicationConfig {
                outbound: vec!["parent-id".to_string()],
                inbound: vec!["parent-id".to_string()],
            },
        };

        let path = temp.path().join("config.json");
        config.save(&path).unwrap();
        let loaded = AgentConfig::load(&path).unwrap();

        assert_eq!(loaded.id, config.id);
        assert_eq!(loaded.name, config.name);
        assert_eq!(loaded.parent_id, config.parent_id);
        assert_eq!(loaded.max_child_depth, config.max_child_depth);
        assert_eq!(loaded.communication.outbound, config.communication.outbound);
    }

    #[test]
    fn test_permissions_save_load() {
        let temp = TempDir::new().unwrap();
        let mut permissions = AgentPermissions {
            agent_id: "test-id".to_string(),
            permissions: HashMap::new(),
            inherited_from: Some("parent-id".to_string()),
        };
        permissions.permissions.insert(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["/usr/bin/git".to_string()],
                    paths: vec![],
                    timeout_ms: Some(300000),
                },
            },
        );

        let path = temp.path().join("permissions.json");
        permissions.save(&path).unwrap();
        let loaded = AgentPermissions::load(&path).unwrap();

        assert_eq!(loaded.agent_id, permissions.agent_id);
        assert!(loaded.is_allowed("exec"));
        assert!(!loaded.is_allowed("network"));
    }

    #[test]
    fn test_default_communication_config() {
        let with_parent = CommunicationConfig::default_with_parent(Some("parent-1"));
        assert_eq!(with_parent.outbound, vec!["parent-1"]);
        assert_eq!(with_parent.inbound, vec!["parent-1"]);

        let without_parent = CommunicationConfig::default_with_parent(None);
        assert!(without_parent.outbound.is_empty());
        assert!(without_parent.inbound.is_empty());
    }
}
