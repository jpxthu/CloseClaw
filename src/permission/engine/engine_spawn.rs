//! Permission Engine - Agent spawn permission validation and caching.

use super::engine_types::{PermissionRequestBody, PermissionResponse};
use crate::agent::config::AgentPermissions;
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
    /// Get the effective permissions for a given agent from the cache.
    ///
    /// Returns `Some(AgentPermissions)` if the agent has been previously validated
    /// and injected into the cache, `None` if not found.
    pub fn get_agent_effective_permissions(&self, agent_id: &str) -> Option<AgentPermissions> {
        let cache = self
            .agent_permissions
            .read()
            .expect("agent_permissions lock poisoned");
        cache.get(agent_id).cloned()
    }

    /// Validate that a child agent's permissions after intersection with parent
    /// are not fully denied, then inject the result into the cache.
    ///
    /// - Computes `child_perms.intersect(parent_perms)` (owner path, `user_perms` is `None`)
    ///   or `child_perms.intersect(parent_perms).intersect(user_perms)` (non-owner path)
    /// - If fully denied → returns `Err(SpawnPermissionError::FullyDenied)` or
    ///   `Err(SpawnPermissionError::FullyDeniedWithUser)` if user context is present
    /// - Otherwise → injects into `self.agent_permissions` cache (agent effective permissions)
    ///   and, when `user_perms` is provided, also injects the intersection result into
    ///   `self.user_effective_permissions` cache (user effective permissions for eval pre-check)
    /// - Idempotent: if already cached, returns `Ok(())` immediately
    pub fn validate_and_inject_spawn(
        &self,
        child_agent_id: &str,
        child_perms: &AgentPermissions,
        parent_perms: &AgentPermissions,
        user_perms: Option<&AgentPermissions>,
        user_id: Option<&str>,
    ) -> Result<(), SpawnPermissionError> {
        let mut agent_cache = self
            .agent_permissions
            .write()
            .expect("agent_permissions lock poisoned");

        // Idempotent: already cached
        if agent_cache.contains_key(child_agent_id) {
            return Ok(());
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

        agent_cache.insert(child_agent_id.to_string(), final_effective.clone());

        // Inject user effective permissions for eval pre-check (non-owner path only)
        if let Some(uid) = user_id {
            let mut user_cache = self
                .user_effective_permissions
                .write()
                .expect("user_effective_permissions lock poisoned");
            user_cache.insert(uid.to_string(), final_effective);
        }

        Ok(())
    }

    /// Check the agent effective permissions cache for a given request body.
    ///
    /// Returns:
    /// - `None` if dimension is unknown (e.g., SlashCommand) or cache miss
    /// - `Some(Denied)` if the dimension is denied
    /// - `None` if the dimension is allowed (continue with normal evaluate)
    pub fn check_agent_effective_permissions(
        &self,
        agent_id: &str,
        body: &PermissionRequestBody,
    ) -> Option<PermissionResponse> {
        let dim = body.dimension_name()?;

        let cache = self
            .agent_permissions
            .read()
            .expect("agent_permissions lock poisoned");

        match cache.get(agent_id) {
            None => None, // cache miss → continue with normal evaluate
            Some(perms) => {
                let allowed = perms
                    .permissions
                    .get(dim)
                    .map(|p| p.allowed)
                    .unwrap_or(false);

                if allowed {
                    None // allowed → continue with normal evaluate
                } else {
                    Some(PermissionResponse::Denied {
                        reason: format!("agent effective permission denied for dimension '{dim}'"),
                        rule: "<agent_effective_permissions>".to_string(),
                        risk_level: super::engine_risk::assess_risk_level(body),
                    })
                }
            }
        }
    }

    /// Get the effective permissions for a given user from the cache.
    ///
    /// Returns `Some(AgentPermissions)` if the user has been previously validated
    /// and injected into the cache, `None` if not found.
    pub fn get_user_effective_permissions(&self, user_id: &str) -> Option<AgentPermissions> {
        let cache = self
            .user_effective_permissions
            .read()
            .expect("user_effective_permissions lock poisoned");
        cache.get(user_id).cloned()
    }

    /// Check the user effective permissions cache for a given request body.
    ///
    /// Returns:
    /// - `None` if dimension is unknown (e.g., SlashCommand) or cache miss
    /// - `Some(Denied)` if the dimension is denied
    /// - `None` if the dimension is allowed (continue with normal evaluate)
    pub fn check_user_effective_permissions(
        &self,
        user_id: &str,
        body: &PermissionRequestBody,
    ) -> Option<PermissionResponse> {
        let dim = body.dimension_name()?;

        let cache = self
            .user_effective_permissions
            .read()
            .expect("user_effective_permissions lock poisoned");

        match cache.get(user_id) {
            None => None, // cache miss → continue with normal evaluate
            Some(perms) => {
                let allowed = perms
                    .permissions
                    .get(dim)
                    .map(|p| p.allowed)
                    .unwrap_or(false);

                if allowed {
                    None // allowed → continue with normal evaluate
                } else {
                    Some(PermissionResponse::Denied {
                        reason: format!("user effective permission denied for dimension '{}'", dim),
                        rule: "<user_effective_permissions>".to_string(),
                        risk_level: super::engine_risk::assess_risk_level(body),
                    })
                }
            }
        }
    }
}
