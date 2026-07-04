//! Permission Engine - Agent spawn permission validation.

use super::engine_types::{PermissionRequestBody, Subject};
use closeclaw_config::agents::AgentPermissions;
use std::collections::HashMap;
use thiserror::Error;

/// Error type for spawn permission validation failures.
#[derive(Debug, Error)]
pub enum SpawnPermissionError {
    #[error(
        "spawn denied: '{child_agent_id}' denied by intersection with parent '{parent_agent_id}'"
    )]
    FullyDenied {
        child_agent_id: String,
        parent_agent_id: String,
    },
    #[error(
        "spawn denied: '{child_agent_id}' denied by three-way intersection (parent: '{parent_agent_id}', user: '{user_id}')"
    )]
    FullyDeniedWithUser {
        child_agent_id: String,
        parent_agent_id: String,
        user_id: String,
    },
}

impl super::engine_eval::PermissionEngine {
    /// Validate that a child agent's permissions after intersection with parent
    /// are not fully denied.
    ///
    /// Computes `child_perms.intersect(parent_perms)` (owner path, `user_perms` is `None`)
    /// or `child_perms.intersect(parent_perms).intersect(user_perms)` (non-owner path).
    /// If fully denied → returns `Err(SpawnPermissionError::FullyDenied)` or
    /// `Err(SpawnPermissionError::FullyDeniedWithUser)` if user context is present.
    pub fn validate_and_inject_spawn(
        &self,
        child_agent_id: &str,
        child_perms: &AgentPermissions,
        parent_perms: &AgentPermissions,
        user_perms: Option<&AgentPermissions>,
        user_id: Option<&str>,
        extra_deny_subjects: Option<&[Subject]>,
    ) -> Result<(), SpawnPermissionError> {
        // Step 0: Extra deny — chain deny subjects override all other checks.
        if let Some(subjects) = extra_deny_subjects {
            let caller = super::engine_types::Caller {
                user_id: user_id.unwrap_or_default().to_string(),
                agent: child_agent_id.to_string(),
                creator_id: String::new(),
            };
            for subject in subjects {
                if subject.matches(&caller) {
                    return Err(SpawnPermissionError::FullyDenied {
                        child_agent_id: child_agent_id.to_string(),
                        parent_agent_id: parent_perms.agent_id.clone(),
                    });
                }
            }
        }

        // Step 1: child ∩ parent
        let effective = child_perms.intersect(parent_perms);

        // Step 2: if user_perms provided, also intersect with user permissions
        let final_effective = match user_perms {
            Some(user) => effective.intersect(user),
            None => effective,
        };

        if final_effective.is_fully_denied() {
            if let Some(uid) = user_id {
                return Err(SpawnPermissionError::FullyDeniedWithUser {
                    child_agent_id: child_agent_id.to_string(),
                    parent_agent_id: parent_perms.agent_id.clone(),
                    user_id: uid.to_string(),
                });
            }
            return Err(SpawnPermissionError::FullyDenied {
                child_agent_id: child_agent_id.to_string(),
                parent_agent_id: parent_perms.agent_id.clone(),
            });
        }

        // No caching — evaluate() computes permissions fresh each time
        Ok(())
    }
}

// --- User permissions evaluation from RuleSet ---

impl super::engine_eval::PermissionEngine {
    /// Evaluate user permissions across all dimensions for a given user and agent,
    /// returning an `AgentPermissions` that can be used for spawn-time intersection.
    ///
    /// Iterates over each permission dimension (exec, file_read, file_write, network,
    /// spawn, tool_call, config_write), constructs a representative request body,
    /// evaluates the User phase (UserAndAgent rules only), and collects the results
    /// into an `AgentPermissions`.
    pub fn evaluate_user_permissions(&self, user_id: &str, agent_id: &str) -> AgentPermissions {
        let dimensions = [
            (
                "exec",
                PermissionRequestBody::CommandExec {
                    agent: agent_id.to_string(),
                    cmd: String::new(),
                    args: Vec::new(),
                },
            ),
            (
                "file_read",
                PermissionRequestBody::FileOp {
                    agent: agent_id.to_string(),
                    path: String::new(),
                    op: "read".to_string(),
                },
            ),
            (
                "file_write",
                PermissionRequestBody::FileOp {
                    agent: agent_id.to_string(),
                    path: String::new(),
                    op: "write".to_string(),
                },
            ),
            (
                "network",
                PermissionRequestBody::NetOp {
                    agent: agent_id.to_string(),
                    host: String::new(),
                    port: 0,
                },
            ),
            (
                "spawn",
                PermissionRequestBody::InterAgentMsg {
                    from: agent_id.to_string(),
                    to: String::new(),
                },
            ),
            (
                "tool_call",
                PermissionRequestBody::ToolCall {
                    agent: agent_id.to_string(),
                    skill: String::new(),
                    method: String::new(),
                },
            ),
            (
                "config_write",
                PermissionRequestBody::ConfigWrite {
                    agent: agent_id.to_string(),
                    config_file: String::new(),
                },
            ),
        ];

        let caller = super::engine_types::Caller {
            user_id: user_id.to_string(),
            agent: agent_id.to_string(),
            creator_id: String::new(),
        };
        let rules = self.rules.clone();

        let mut permissions = HashMap::with_capacity(dimensions.len());
        for (dim, body) in &dimensions {
            let candidates = self.collect_user_agent_candidates(&caller, agent_id, &rules);
            let result = self.match_rules(&candidates, &rules, &caller, body);
            let allowed = matches!(
                result,
                Some(super::engine_types::PermissionResponse::Allowed { .. })
            );
            permissions.insert(
                dim.to_string(),
                closeclaw_config::agents::ActionPermission {
                    allowed,
                    limits: closeclaw_config::agents::PermissionLimits::default(),
                },
            );
        }

        AgentPermissions {
            agent_id: agent_id.to_string(),
            permissions,
            inherited_from: None,
        }
    }
}
