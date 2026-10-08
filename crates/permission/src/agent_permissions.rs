//! Agent permission domain types and the configuration port.
//!
//! Permission-owned mirror of the agent permission model (previously
//! consumed from `closeclaw_config::agents`): the engine evaluates
//! spawn-chain intersections and full-denial checks against these
//! self-held types. Concrete loading of `permissions.json` stays on the
//! config side; the composition root (daemon) adapts the config provider
//! to [`AgentPermissionProvider`].

use std::collections::HashMap;

/// The eight permission dimensions checked by
/// [`AgentPermissions::intersect`] and [`AgentPermissions::is_fully_denied`].
const DIMENSIONS: [&str; 8] = [
    "exec",
    "file_read",
    "file_write",
    "network",
    "spawn",
    "tool_call",
    "config_write",
    "message",
];

/// Permission limits for a single action category.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionLimits {
    /// Allowed commands (for exec).
    pub commands: Vec<String>,
    /// Allowed paths (for file_read/file_write).
    pub paths: Vec<String>,
    /// Timeout limit in milliseconds (for exec).
    pub timeout_ms: Option<u64>,
}

/// Permissions for a single action category.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionPermission {
    /// Whether this action is allowed.
    pub allowed: bool,
    /// Optional limits when allowed.
    pub limits: PermissionLimits,
}

/// Full permissions for an agent (permission-domain view of
/// `permissions.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentPermissions {
    /// Agent identifier these permissions apply to.
    pub agent_id: String,
    /// Permission rules by action category.
    pub permissions: HashMap<String, ActionPermission>,
    /// ID of the agent from which these permissions are inherited.
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
    /// Eight dimensions: exec, file_read, file_write, network, spawn,
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
        let mut permissions = HashMap::with_capacity(DIMENSIONS.len());

        for &dim in &DIMENSIONS {
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
        !DIMENSIONS
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

/// Port for lazily providing agent permissions by agent ID.
///
/// Permission-side abstraction over the configuration source: the engine
/// consumes `&dyn AgentPermissionProvider` per call; the composition root
/// (daemon) supplies the production implementation adapting the config
/// crate's lazy loader.
pub trait AgentPermissionProvider: Send + Sync {
    /// Get the permissions for the given agent, or `None` if the agent
    /// has no custom permission configuration.
    fn get(&self, agent_id: &str) -> Option<AgentPermissions>;
}
