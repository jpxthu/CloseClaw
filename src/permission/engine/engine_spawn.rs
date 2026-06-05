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
}

impl super::engine_eval::PermissionEngine {
    /// Validate that a child agent's permissions after intersection with parent
    /// are not fully denied, then inject the result into the cache.
    ///
    /// - Computes `child_perms.intersect(parent_perms)`
    /// - If fully denied → returns `Err(SpawnPermissionError::FullyDenied)`
    /// - Otherwise → injects into `self.agent_permissions` cache
    /// - Idempotent: if already cached, returns `Ok(())` immediately
    pub fn validate_and_inject_spawn(
        &self,
        child_agent_id: &str,
        child_perms: &AgentPermissions,
        parent_perms: &AgentPermissions,
    ) -> Result<(), SpawnPermissionError> {
        let mut cache = self
            .agent_permissions
            .write()
            .expect("agent_permissions lock poisoned");

        // Idempotent: already cached
        if cache.contains_key(child_agent_id) {
            return Ok(());
        }

        let effective = child_perms.intersect(parent_perms);

        if effective.is_fully_denied() {
            return Err(SpawnPermissionError::FullyDenied {
                child_agent_id: child_agent_id.to_string(),
                parent_agent_id: parent_perms.agent_id.clone(),
            });
        }

        cache.insert(child_agent_id.to_string(), effective);
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
}
