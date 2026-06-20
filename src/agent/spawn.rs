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

/// Internal result from resolving parent depth and max spawn budget.
struct ResolvedParentDepth {
    parent_depth: u32,
    parent_effective_budget: u32,
}

/// Internal result from resolving target agent configuration.
struct ResolvedTarget {
    target_id: Option<String>,
    max_children: u32,
    allow_agents: Vec<String>,
    require_agent_id: bool,
    target_config: Option<ResolvedAgentConfig>,
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
        // ① Depth check — read parent depth + config, validate budget.
        let parent = self.resolve_parent_depth(parent_session_id).await?;

        // ② AgentId fallback + config resolution (single lock block).
        let resolved = self
            .resolve_target_config(parent_session_id, target_agent_id)
            .await?;

        // ③ Concurrency check.
        self.check_concurrency(parent_session_id, resolved.max_children)
            .await?;

        // ④ Whitelist check.
        if let Some(ref tid) = resolved.target_id {
            self.check_whitelist(tid, &resolved.allow_agents)?;
        }

        // ⑤ require_agent_id check — must come after concurrency/whitelist.
        if resolved.require_agent_id && resolved.target_id.is_none() {
            return Err(SpawnError::AgentIdRequired);
        }
        let target_id = resolved.target_id.ok_or(SpawnError::AgentIdRequired)?;

        // ⑥ Compute effective_max_spawn_depth for the child.
        let child_max_spawn_depth = resolved
            .target_config
            .as_ref()
            .map(|c| c.subagents.max_spawn_depth)
            .unwrap_or(1);
        let effective_max =
            child_max_spawn_depth.min(parent.parent_effective_budget.saturating_sub(1));
        let child_depth = parent.parent_depth + 1;
        if child_depth > effective_max {
            return Err(SpawnError::DepthExceeded {
                current: child_depth,
                max: effective_max,
            });
        }

        let config = resolved
            .target_config
            .ok_or(SpawnError::ConfigNotFound(target_id))?;
        Ok(SpawnValidationResult {
            config,
            effective_max_spawn_depth: effective_max,
        })
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Resolve the parent session's depth and maximum spawn budget.
    /// Reads the parent agent config to get `max_spawn_depth`, then
    /// compares with `get_effective_max_spawn_depth` from session manager.
    async fn resolve_parent_depth(
        &self,
        parent_session_id: &str,
    ) -> Result<ResolvedParentDepth, SpawnError> {
        let parent_depth = self
            .session_manager
            .get_session_depth(parent_session_id)
            .await
            .unwrap_or(0);

        let parent_agent_id = self
            .session_manager
            .get_chat_id(parent_session_id)
            .await
            .unwrap_or_default();

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

        Ok(ResolvedParentDepth {
            parent_depth,
            parent_effective_budget,
        })
    }

    /// Resolve the target agent configuration under a single lock block.
    /// Handles the agentId fallback: if no `target_agent_id` is provided,
    /// falls back to the parent config's `default_child_agent`.
    async fn resolve_target_config(
        &self,
        parent_session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<ResolvedTarget, SpawnError> {
        let parent_agent_id = self
            .session_manager
            .get_chat_id(parent_session_id)
            .await
            .unwrap_or_default();

        // Single lock acquisition — agentId fallback + parent config read.
        // ConfigManager.agents uses std::sync::RwLock whose read guard is
        // !Send. All lock-scoped work must complete before any .await.
        let (target_id, max_children, allow_agents, require_agent_id, target_config) = {
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
                    (
                        target_agent_id.map(|s| s.to_string()),
                        false,
                        5u32,
                        vec!["*".to_string()],
                    )
                };

            let target_config = target_id.as_ref().and_then(|id| agents.get(id).cloned());

            (
                target_id,
                max_children,
                allow_agents,
                require_agent_id,
                target_config,
            )
        };

        Ok(ResolvedTarget {
            target_id,
            max_children,
            allow_agents,
            require_agent_id,
            target_config,
        })
    }

    /// Check that the parent has not reached its maximum concurrent children.
    async fn check_concurrency(
        &self,
        parent_session_id: &str,
        max_children: u32,
    ) -> Result<(), SpawnError> {
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
        Ok(())
    }

    /// Check that the target agent is in the parent's allowlist.
    fn check_whitelist(&self, target_id: &str, allow_agents: &[String]) -> Result<(), SpawnError> {
        if !allow_agents.iter().any(|a| a == "*" || a == target_id) {
            return Err(SpawnError::AgentNotAllowed {
                agent_id: target_id.to_string(),
            });
        }
        Ok(())
    }
}
