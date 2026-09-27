//! Permission types for agents (permissions.json).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Permission types (unchanged) ────────────────────────────────────────

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
    /// Check if a specific action is permitted.
    pub fn is_allowed(&self, action: &str) -> bool {
        self.permissions
            .get(action)
            .map(|p| p.allowed)
            .unwrap_or(false)
    }

    /// Compute the intersection of this agent's permissions with a parent's.
    ///
    /// Eight dimensions: command, file_read, file_write, network, spawn,
    /// tool_call, config_write, message.
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
            "message",
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

    /// Returns true if all eight permission dimensions are denied or absent.
    pub fn is_fully_denied(&self) -> bool {
        ![
            "exec",
            "file_read",
            "file_write",
            "network",
            "spawn",
            "tool_call",
            "config_write",
            "message",
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
