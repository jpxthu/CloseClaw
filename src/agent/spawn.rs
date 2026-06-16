//! Spawn control — validates spawn requests against agent configuration.

use crate::config::agents::ResolvedAgentConfig;
use crate::config::ConfigManager;
use crate::gateway::SessionManager;
use std::sync::Arc;
use thiserror::Error;

/// Result of a successful spawn validation, containing the resolved
/// target config and the effective max spawn depth for the child.
#[derive(Debug, Clone)]
pub struct SpawnValidationResult {
    /// Resolved configuration of the target agent.
    pub config: ResolvedAgentConfig,
    /// Remaining spawn depth the child has after this spawn.
    /// Computed as `min(child.max_spawn_depth, parent.max_spawn_depth - 1)`.
    pub effective_remaining_depth: u32,
}

/// Errors returned by SpawnController validation.
#[derive(Debug, Error)]
pub enum SpawnError {
    #[error(
        "spawn depth exhausted: child has no remaining spawn capability (remaining={remaining})"
    )]
    DepthExhausted { remaining: u32 },
    #[error("max concurrent children reached: {current} >= {max}")]
    MaxChildrenReached { current: usize, max: u32 },
    #[error("agent '{agent_id}' not in allowlist")]
    AgentNotAllowed { agent_id: String },
    #[error("agentId is required by configuration")]
    AgentIdRequired,
    #[error("agent config not found: {0}")]
    ConfigNotFound(String),
    #[error("spawn permission denied for agent '{agent_id}': {reason}")]
    PermissionDenied { agent_id: String, reason: String },
}

/// Validates spawn requests and delegates child session creation.
pub struct SpawnController {
    config_manager: Arc<ConfigManager>,
    session_manager: Arc<SessionManager>,
}

impl SpawnController {
    pub fn new(config_manager: Arc<ConfigManager>, session_manager: Arc<SessionManager>) -> Self {
        Self {
            config_manager,
            session_manager,
        }
    }

    /// Validate a spawn request.
    ///
    /// Returns a [`SpawnValidationResult`] with the target agent's resolved
    /// config and the child's remaining spawn depth, or a
    /// [`SpawnError`] on failure.
    pub async fn validate(
        &self,
        parent_session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<SpawnValidationResult, SpawnError> {
        // 1. Determine parent agent_id from session manager
        let parent_agent_id = self
            .session_manager
            .get_chat_id(parent_session_id)
            .await
            .unwrap_or_default();

        // 3-5. Load parent config and resolve the target id under the lock.
        // ConfigManager.agents uses std::sync::RwLock, whose read guard is
        // !Send. We must not hold it across an .await, so all lock-scoped
        // work is performed inside this block and the guard is dropped
        // before any await below.
        let (target_id, max_spawn_depth, max_children, allow_agents, target_config) = {
            let agents = self
                .config_manager
                .agents
                .read()
                .expect("RwLock for agents was poisoned");
            let parent_config = agents.get(&parent_agent_id);

            let (target_id, require_agent_id, max_spawn_depth, max_children, allow_agents) =
                if let Some(pc) = parent_config {
                    let sc = &pc.subagents;
                    (
                        target_agent_id
                            .map(|s| s.to_string())
                            .or_else(|| sc.default_child_agent.clone()),
                        sc.require_agent_id,
                        sc.max_spawn_depth,
                        sc.max_children,
                        sc.allow_agents.clone(),
                    )
                } else {
                    // No parent config found — use design-doc defaults
                    (
                        target_agent_id.map(|s| s.to_string()),
                        false,
                        1u32,
                        5u32,
                        vec!["*".to_string()],
                    )
                };

            // 5. Check require_agent_id
            if require_agent_id && target_id.is_none() {
                return Err(SpawnError::AgentIdRequired);
            }

            let target_id = target_id.ok_or(SpawnError::AgentIdRequired)?;

            // Pre-clone the target config so we don't need the lock after this block.
            let target_config = agents.get(&target_id).cloned();

            (
                target_id,
                max_spawn_depth,
                max_children,
                allow_agents,
                target_config,
            )
        };

        // 6. Depth check — reject if the parent itself has reached its depth
        //    limit.  The child's effective remaining spawn depth is
        //    min(child.max_spawn_depth, parent.max_spawn_depth - parent_depth - 1).
        //    A value of 0 is still valid (child is allowed to exist, it just
        //    cannot spawn further).
        let parent_depth = self
            .session_manager
            .get_session_depth(parent_session_id)
            .await
            .unwrap_or(0);
        if parent_depth >= max_spawn_depth {
            return Err(SpawnError::DepthExhausted { remaining: 0 });
        }
        let child_max_spawn_depth = target_config
            .as_ref()
            .map(|c| c.subagents.max_spawn_depth)
            .unwrap_or(1);
        let effective_remaining =
            child_max_spawn_depth.min(max_spawn_depth.saturating_sub(parent_depth + 1));

        // 7. Concurrency check
        // No lock held here — safe to await.
        let active = self
            .session_manager
            .count_active_children(parent_session_id)
            .await;
        if active as u32 >= max_children {
            return Err(SpawnError::MaxChildrenReached {
                current: active,
                max: max_children,
            });
        }

        // 8. Whitelist check
        if !allow_agents.iter().any(|a| a == "*" || a == &target_id) {
            return Err(SpawnError::AgentNotAllowed {
                agent_id: target_id,
            });
        }

        // 9. Return validation result with child's remaining spawn depth
        let config = target_config.ok_or(SpawnError::ConfigNotFound(target_id))?;
        Ok(SpawnValidationResult {
            config,
            effective_remaining_depth: effective_remaining,
        })
    }
}
