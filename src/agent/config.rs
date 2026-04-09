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

    /// Check if communication to `target_id` is allowed.
    pub fn can_send_to(&self, target_id: &str) -> bool {
        self.outbound.iter().any(|id| id == target_id || id == "*")
    }

    /// Check if communication from `source_id` is allowed.
    pub fn can_receive_from(&self, source_id: &str) -> bool {
        self.inbound.iter().any(|id| id == source_id || id == "*")
    }
}

/// Result of a communication permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommunicationCheckResult {
    /// Communication is allowed.
    Allowed,
    /// Source agent not in target's inbound list.
    SourceNotInTargetInbound,
    /// Target agent not in source's outbound list.
    TargetNotInSourceOutbound,
}

/// Check if communication from source to target is allowed.
/// Returns the result of the central arbiter logic.
pub fn check_communication_allowed(
    source_config: &AgentConfig,
    target_config: &AgentConfig,
) -> CommunicationCheckResult {
    // Step 1: Check if target is in source's outbound list
    if !source_config.communication.can_send_to(&target_config.id) {
        return CommunicationCheckResult::TargetNotInSourceOutbound;
    }

    // Step 2: Check if source is in target's inbound list
    if !target_config
        .communication
        .can_receive_from(&source_config.id)
    {
        return CommunicationCheckResult::SourceNotInTargetInbound;
    }

    // Both checks passed — communication allowed
    CommunicationCheckResult::Allowed
}

/// Result of a max_depth permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaxDepthCheckResult {
    /// Agent can spawn children at this depth.
    Allowed {
        current_depth: u32,
        max_allowed: u32,
    },
    /// Agent cannot spawn: would exceed max_child_depth.
    ExceedsMaxDepth {
        current_depth: u32,
        max_child_depth: u32,
    },
}

/// Check if an agent can spawn a child at the proposed depth.
///
/// `get_parent` is a callback that returns the parent agent's config given its ID.
/// Returns the result of the depth check.
pub fn check_max_depth<F>(agent_config: &AgentConfig, get_parent: F) -> MaxDepthCheckResult
where
    F: Fn(&str) -> Option<AgentConfig>,
{
    // Calculate current depth by traversing up the parent chain
    let mut current_depth = 0u32;
    let mut current_parent_id = agent_config.parent_id.clone();

    while let Some(parent_id) = current_parent_id {
        if parent_id.is_empty() {
            break;
        }
        if let Some(parent) = get_parent(&parent_id) {
            current_depth += 1;
            current_parent_id = parent.parent_id.clone();
        } else {
            break;
        }
    }

    let max_child_depth = agent_config.max_child_depth;

    if current_depth >= max_child_depth {
        MaxDepthCheckResult::ExceedsMaxDepth {
            current_depth,
            max_child_depth,
        }
    } else {
        // max_child_depth is the absolute maximum depth from this agent as root
        // So the child can go up to max_child_depth - 1 more levels
        let max_allowed = max_child_depth.saturating_sub(1);
        MaxDepthCheckResult::Allowed {
            current_depth,
            max_allowed,
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
    /// Wait timeout for graceful child agent shutdown (seconds).
    /// None means use registry-level default (30s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_timeout_secs: Option<u64>,
    /// Grace period after wait timeout before SIGTERM/SIGKILL (seconds).
    /// None means use registry-level default (10s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace_period_secs: Option<u64>,
}

fn default_max_child_depth() -> u32 {
    3
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            parent_id: None,
            max_child_depth: default_max_child_depth(),
            created_at: chrono::Utc::now(),
            state: AgentConfigState::default(),
            communication: CommunicationConfig::default(),
            wait_timeout_secs: None,
            grace_period_secs: None,
        }
    }
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
            ..Default::default()
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

    #[test]
    fn test_communication_allowed() {
        let parent = AgentConfig {
            id: "parent-1".to_string(),
            name: "Parent".to_string(),
            parent_id: None,
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: CommunicationConfig {
                outbound: vec!["child-1".to_string()],
                inbound: vec!["child-1".to_string()],
            },
            ..Default::default()
        };

        let child = AgentConfig {
            id: "child-1".to_string(),
            name: "Child".to_string(),
            parent_id: Some("parent-1".to_string()),
            max_child_depth: 1,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: CommunicationConfig::default_with_parent(Some("parent-1")),
            ..Default::default()
        };

        // Parent -> Child should be allowed
        let result = check_communication_allowed(&parent, &child);
        assert_eq!(result, CommunicationCheckResult::Allowed);

        // Child -> Parent should be allowed
        let result = check_communication_allowed(&child, &parent);
        assert_eq!(result, CommunicationCheckResult::Allowed);
    }

    #[test]
    fn test_communication_denied_outbound() {
        let agent_a = AgentConfig {
            id: "agent-a".to_string(),
            name: "Agent A".to_string(),
            parent_id: None,
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: CommunicationConfig {
                outbound: vec!["agent-b".to_string()],
                inbound: vec!["agent-b".to_string()],
            },
            ..Default::default()
        };

        let agent_c = AgentConfig {
            id: "agent-c".to_string(),
            name: "Agent C".to_string(),
            parent_id: None,
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: CommunicationConfig {
                outbound: vec![],
                inbound: vec![],
            },
            ..Default::default()
        };

        // Agent A -> Agent C: A's outbound doesn't contain C
        let result = check_communication_allowed(&agent_a, &agent_c);
        assert_eq!(result, CommunicationCheckResult::TargetNotInSourceOutbound);
    }

    #[test]
    fn test_communication_denied_inbound() {
        let agent_a = AgentConfig {
            id: "agent-a".to_string(),
            name: "Agent A".to_string(),
            parent_id: None,
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: CommunicationConfig {
                outbound: vec!["agent-b".to_string()],
                inbound: vec!["agent-b".to_string()],
            },
            ..Default::default()
        };

        let agent_b = AgentConfig {
            id: "agent-b".to_string(),
            name: "Agent B".to_string(),
            parent_id: None,
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: CommunicationConfig {
                outbound: vec![],
                inbound: vec![], // B doesn't accept inbound from anyone
            },
            ..Default::default()
        };

        // Agent A -> Agent B: A's outbound contains B, but B's inbound doesn't contain A
        let result = check_communication_allowed(&agent_a, &agent_b);
        assert_eq!(result, CommunicationCheckResult::SourceNotInTargetInbound);
    }

    #[test]
    fn test_max_depth_allowed() {
        // Root agent with max_child_depth=3, currently at depth 0
        let root = AgentConfig {
            id: "root".to_string(),
            name: "Root".to_string(),
            parent_id: None,
            max_child_depth: 3,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: Default::default(),
            ..Default::default()
        };

        // No parents, so depth = 0, max_child_depth = 3
        // Can spawn (0 < 3), max_allowed for child = 2
        let result = check_max_depth(&root, |_: &str| None);
        match result {
            MaxDepthCheckResult::Allowed {
                current_depth,
                max_allowed,
            } => {
                assert_eq!(current_depth, 0);
                assert_eq!(max_allowed, 2); // 3 - 1
            }
            _ => panic!("expected Allowed"),
        }
    }

    #[test]
    fn test_max_depth_exceeded() {
        // Agent at depth 3, max_child_depth=2 (already exceeded!)
        let leaf = AgentConfig {
            id: "leaf".to_string(),
            name: "Leaf".to_string(),
            parent_id: Some("parent".to_string()),
            max_child_depth: 2,
            created_at: Utc::now(),
            state: AgentConfigState::Running,
            communication: Default::default(),
            ..Default::default()
        };

        // Simulate: root -> child1 -> child2 -> leaf (depth 3)
        let get_parent = |id: &str| match id {
            "parent" => Some(AgentConfig {
                id: "parent".to_string(),
                name: "Parent".to_string(),
                parent_id: Some("grandparent".to_string()),
                max_child_depth: 2,
                created_at: Utc::now(),
                state: AgentConfigState::Running,
                communication: Default::default(),
                ..Default::default()
            }),
            "grandparent" => Some(AgentConfig {
                id: "grandparent".to_string(),
                name: "Grandparent".to_string(),
                parent_id: None,
                max_child_depth: 3,
                created_at: Utc::now(),
                state: AgentConfigState::Running,
                communication: Default::default(),
                ..Default::default()
            }),
            _ => None,
        };

        let result = check_max_depth(&leaf, get_parent);
        match result {
            MaxDepthCheckResult::ExceedsMaxDepth {
                current_depth,
                max_child_depth,
            } => {
                // leaf has 2 ancestors (parent + grandparent) = depth 2
                assert_eq!(current_depth, 2);
                assert_eq!(max_child_depth, 2);
            }
            _ => panic!("expected ExceedsMaxDepth"),
        }
    }
}
