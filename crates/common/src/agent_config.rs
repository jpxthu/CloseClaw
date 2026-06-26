//! Agent configuration types — config.json and permissions.json structures
//! for per-agent config files.
//!
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::bootstrap::BootstrapMode;

/// Agent's own configuration (stored as config.json in the agent's directory).
///
/// Permissions are stored in a separate `permissions.json` file, not inline
/// in `config.json` (per design doc).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Unique identifier for this agent.
    pub id: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: Option<String>,
    /// Parent agent ID (if this agent was spawned by another).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Default LLM model for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Working directory path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Directory for bootstrap files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_dir: Option<String>,
    /// Bootstrap file loading mode.
    #[serde(default = "default_bootstrap_mode")]
    pub bootstrap_mode: BootstrapMode,
    /// Available skill names; `["*"]` means all skills are available.
    #[serde(default = "default_all")]
    pub skills: Vec<String>,
    /// Available tool names whitelist.
    #[serde(default = "default_all")]
    pub tools: Vec<String>,
    /// Disallowed tool names blacklist.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Sub-agent spawn control parameters.
    #[serde(default)]
    pub subagents: SubagentsConfig,
    /// Memory subsystem configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
}

fn default_all() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_bootstrap_mode() -> BootstrapMode {
    BootstrapMode::Full
}

/// Sub-agent spawn control configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsConfig {
    /// Whitelist of allowed target agent IDs; `["*"]` means no restriction.
    #[serde(default = "default_all")]
    pub allow_agents: Vec<String>,
    /// Whether agentId must be explicitly specified when spawning.
    #[serde(default)]
    pub require_agent_id: bool,
    /// Maximum nested spawn depth.
    #[serde(default = "default_max_spawn_depth")]
    pub max_spawn_depth: u32,
    /// Maximum concurrent active child sessions.
    #[serde(default = "default_max_children")]
    pub max_children: u32,
    /// Default child agent ID (used when spawn omits agentId).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_child_agent: Option<String>,
    /// Model override for child agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Default for SubagentsConfig {
    fn default() -> Self {
        Self {
            allow_agents: default_all(),
            require_agent_id: false,
            max_spawn_depth: default_max_spawn_depth(),
            max_children: default_max_children(),
            default_child_agent: None,
            model: None,
        }
    }
}

fn default_max_spawn_depth() -> u32 {
    1
}

fn default_max_children() -> u32 {
    5
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: None,
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: default_bootstrap_mode(),
            skills: default_all(),
            tools: default_all(),
            disallowed_tools: Vec::new(),
            subagents: SubagentsConfig::default(),
            memory: None,
        }
    }
}

/// Memory subsystem configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_searcher: Option<ActiveSearcherOverride>,
}

/// Active-searcher overrides — all fields optional.
/// Missing fields fall back to defaults (model from agent global).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSearcherOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_summary_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_entity_hits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k_events: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_turns: Option<usize>,
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

    /// Compute the intersection of this agent's permissions with a parent's.
    ///
    /// Seven dimensions: exec, file_read, file_write, network, spawn,
    /// tool_call, config_write.
    ///
    /// - Both Allow → Allow
    /// - Either Deny or absent → Deny
    /// - Result `agent_id` = self.agent_id, `inherited_from` = Some(parent.agent_id)
    /// - Limits: commands/paths → set intersection; timeout_ms → min;
    ///   Deny dimensions get default limits.
    /// - None means no restriction: both None → None, one None → other's Some,
    ///   both Some → min.
    pub fn intersect(&self, parent: &AgentPermissions) -> Self {
        let dimensions = [
            "exec",
            "file_read",
            "file_write",
            "network",
            "spawn",
            "tool_call",
            "config_write",
        ];

        let mut permissions = HashMap::with_capacity(dimensions.len());

        for &dim in &dimensions {
            let self_perm = self.permissions.get(dim);
            let parent_perm = parent.permissions.get(dim);

            let self_allowed = self_perm.map(|p| p.allowed).unwrap_or(false);
            let parent_allowed = parent_perm.map(|p| p.allowed).unwrap_or(false);

            if self_allowed && parent_allowed {
                let self_limits = self_perm.map(|p| &p.limits);
                let parent_limits = parent_perm.map(|p| &p.limits);
                let limits = PermissionLimits {
                    commands: intersect_vec(
                        self_limits.map(|l| &l.commands),
                        parent_limits.map(|l| &l.commands),
                    ),
                    paths: intersect_vec(
                        self_limits.map(|l| &l.paths),
                        parent_limits.map(|l| &l.paths),
                    ),
                    timeout_ms: intersect_option_min(
                        self_limits.and_then(|l| l.timeout_ms),
                        parent_limits.and_then(|l| l.timeout_ms),
                    ),
                };
                permissions.insert(
                    dim.to_string(),
                    ActionPermission {
                        allowed: true,
                        limits,
                    },
                );
            } else {
                permissions.insert(
                    dim.to_string(),
                    ActionPermission {
                        allowed: false,
                        limits: PermissionLimits::default(),
                    },
                );
            }
        }

        Self {
            agent_id: self.agent_id.clone(),
            permissions,
            inherited_from: Some(parent.agent_id.clone()),
        }
    }

    /// Returns true if all seven permission dimensions are denied or absent.
    pub fn is_fully_denied(&self) -> bool {
        ![
            "exec",
            "file_read",
            "file_write",
            "network",
            "spawn",
            "tool_call",
            "config_write",
        ]
        .iter()
        .any(|&dim| self.permissions.get(dim).is_some_and(|p| p.allowed))
    }
}

/// Set intersection: if both have some, return common elements;
/// if either is None (no restriction), take the other's value;
/// if both None → None.
pub(crate) fn intersect_vec<T: Eq + std::hash::Hash + Clone>(
    a: Option<&Vec<T>>,
    b: Option<&Vec<T>>,
) -> Vec<T> {
    match (a, b) {
        (Some(a), Some(b)) => a.iter().filter(|item| b.contains(item)).cloned().collect(),
        (Some(a), None) | (None, Some(a)) => a.clone(),
        (None, None) => Vec::new(),
    }
}

/// Minimum of two optional values; if either is None (no restriction),
/// the result is the other's value.
pub(crate) fn intersect_option_min(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}
