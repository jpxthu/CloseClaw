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
    /// Effective max spawn depth the child may use.
    /// Computed as `min(child.max_spawn_depth, parent.max_spawn_depth - 1)`.
    pub effective_max_spawn_depth: u32,
}

/// Errors returned by SpawnController validation.
#[derive(Debug, Error)]
pub enum SpawnError {
    #[error("spawn depth limit exceeded: current depth {current} >= max {max}")]
    DepthExceeded { current: u32, max: u32 },
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
    /// config and the effective max spawn depth for the child, or a
    /// [`SpawnError`] on failure.
    pub async fn validate(
        &self,
        parent_session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<SpawnValidationResult, SpawnError> {
        // 1. Get parent depth — no lock needed, only reads runtime state.
        let parent_depth = self
            .session_manager
            .get_session_depth(parent_session_id)
            .await
            .unwrap_or(0);

        // 2. Determine parent agent_id from session manager.
        let parent_agent_id = self
            .session_manager
            .get_chat_id(parent_session_id)
            .await
            .unwrap_or_default();

        // 3. Depth check FIRST — design doc requires depth to be evaluated
        //    before agentId resolution.  We read the parent's max_spawn_depth
        //    from the config (read lock held only for the lookup, dropped
        //    immediately).  This ensures depth=0 always returns DepthExceeded
        //    rather than falling through to AgentIdRequired.
        let parent_max_spawn_depth = {
            let agents = self
                .config_manager
                .agents
                .read()
                .expect("RwLock for agents was poisoned");
            agents
                .get(&parent_agent_id)
                .map(|pc| pc.subagents.max_spawn_depth)
                .unwrap_or(1u32)
        };

        let parent_effective_budget = self
            .session_manager
            .get_effective_max_spawn_depth(parent_session_id)
            .await
            .unwrap_or(parent_max_spawn_depth);

        if parent_effective_budget == 0 {
            return Err(SpawnError::DepthExceeded {
                current: parent_depth + 1,
                max: 0,
            });
        }

        // 4-8. Load parent config and resolve the target id under the lock.
        // ConfigManager.agents uses std::sync::RwLock, whose read guard is
        // !Send. We must not hold it across an .await, so all lock-scoped
        // work is performed inside this block and the guard is dropped
        // before any await below.
        let (target_id, max_children, allow_agents, target_config) = {
            let agents = self
                .config_manager
                .agents
                .read()
                .expect("RwLock for agents was poisoned");
            let parent_config = agents.get(&parent_agent_id);

            let (target_id, require_agent_id, max_children, allow_agents) =
                if let Some(pc) = parent_config {
                    let sc = &pc.subagents;
                    (
                        target_agent_id
                            .map(|s| s.to_string())
                            .or_else(|| sc.default_child_agent.clone()),
                        sc.require_agent_id,
                        sc.max_children,
                        sc.allow_agents.clone(),
                    )
                } else {
                    // No parent config found — use design-doc defaults
                    (
                        target_agent_id.map(|s| s.to_string()),
                        false,
                        5u32,
                        vec!["*".to_string()],
                    )
                };

            // 4a. agentId fallback already done above via or_else(default_child_agent).

            // 5. require_agent_id check
            if require_agent_id && target_id.is_none() {
                return Err(SpawnError::AgentIdRequired);
            }

            let target_id = target_id.ok_or(SpawnError::AgentIdRequired)?;

            // Pre-clone the target config so we don't need the lock after this block.
            let target_config = agents.get(&target_id).cloned();

            (target_id, max_children, allow_agents, target_config)
        };

        // 6. Concurrency check
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

        // 7. Whitelist check
        if !allow_agents.iter().any(|a| a == "*" || a == &target_id) {
            return Err(SpawnError::AgentNotAllowed {
                agent_id: target_id,
            });
        }

        // 8. Compute effective_max_spawn_depth for the child and return result.
        //    depth was already validated in step 3 (parent budget != 0);
        //    now verify child_depth <= effective_max.
        let child_max_spawn_depth = target_config
            .as_ref()
            .map(|c| c.subagents.max_spawn_depth)
            .unwrap_or(1);
        let effective_max = child_max_spawn_depth.min(parent_effective_budget.saturating_sub(1));
        let child_depth = parent_depth + 1;
        if child_depth > effective_max {
            return Err(SpawnError::DepthExceeded {
                current: child_depth,
                max: effective_max,
            });
        }

        let config = target_config.ok_or(SpawnError::ConfigNotFound(target_id))?;
        Ok(SpawnValidationResult {
            config,
            effective_max_spawn_depth: effective_max,
        })
    }
}
