//! SpawnController — validates spawn requests against agent configuration.
//!
//! Migrated from `closeclaw-agent` (Step 1.2) to enforce the design doc
//! architecture boundary: agent crate is pure configuration, SpawnController
//! lives in the gateway (Session) crate.

use closeclaw_config::agents::ResolvedAgentConfig;
use closeclaw_config::ConfigManager;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_helpers::collect_chain_deny_subjects;
use closeclaw_permission::engine::engine_helpers::collect_chain_effective_permissions;
use std::sync::Arc;
use thiserror::Error;

use super::SessionManager;

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
    permission_engine: Arc<PermissionEngine>,
}

/// Internal result from resolving parent depth and max spawn budget.
struct ResolvedParentDepth {
    parent_effective_budget: u32,
}

/// Parent agent config fields needed for concurrency/whitelist checks.
struct ParentSpawnConfig {
    max_children: u32,
    allow_agents: Vec<String>,
    require_agent_id: bool,
}

/// Result from resolving target agent configuration (agentId fallback).
struct ResolvedTarget {
    target_id: Option<String>,
    target_config: Option<ResolvedAgentConfig>,
}

impl SpawnController {
    pub fn new(
        config_manager: Arc<ConfigManager>,
        session_manager: Arc<SessionManager>,
        permission_engine: Arc<PermissionEngine>,
    ) -> Self {
        Self {
            config_manager,
            session_manager,
            permission_engine,
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
        // ① Depth check.
        let parent_agent_id = self
            .session_manager
            .get_chat_id(parent_session_id)
            .await
            .unwrap_or_default();
        let parent = self
            .resolve_parent_depth(parent_session_id, &parent_agent_id)
            .await?;

        // ② Read parent config for concurrency/whitelist/requireAgentId.
        let parent_cfg = self.read_parent_config(&parent_agent_id).await?;

        // ③ Concurrency check.
        self.check_concurrency(parent_session_id, parent_cfg.max_children)
            .await?;

        // ④ AgentId fallback + target config resolution.
        let resolved = self
            .resolve_target_config(&parent_agent_id, target_agent_id)
            .await?;

        // ⑤ Whitelist check (on resolved target_id, after fallback).
        if let Some(ref tid) = resolved.target_id {
            self.check_whitelist(tid, &parent_cfg.allow_agents)?;
        }

        // ⑥ require_agent_id check — must come after concurrency/whitelist.
        if parent_cfg.require_agent_id && resolved.target_id.is_none() {
            return Err(SpawnError::AgentIdRequired);
        }
        let target_id = resolved.target_id.ok_or(SpawnError::AgentIdRequired)?;

        // ⑦ Permission check: validate child permissions via intersection
        //    with parent effective permissions (design doc §⑥).
        let config = resolved
            .target_config
            .ok_or(SpawnError::ConfigNotFound(target_id))?
            .clone();
        self.validate_permissions(&config, parent_session_id)
            .await?;

        // ⑧ Compute effective_max_spawn_depth and validate child depth.
        let effective_max =
            self.compute_effective_max_depth(parent.parent_effective_budget, Some(&config))?;

        Ok(SpawnValidationResult {
            config,
            effective_max_spawn_depth: effective_max,
        })
    }

    /// Compute the effective maximum spawn depth for a child session.
    fn compute_effective_max_depth(
        &self,
        parent_effective_budget: u32,
        target_config: Option<&ResolvedAgentConfig>,
    ) -> Result<u32, SpawnError> {
        let child_max_depth = target_config
            .and_then(|c| c.subagents.max_spawn_depth)
            .unwrap_or(1);
        let effective_max = child_max_depth.min(parent_effective_budget.saturating_sub(1));
        Ok(effective_max)
    }

    /// Validate spawn permissions: intersect child permissions with parent
    /// effective permissions.
    ///
    /// Returns `Ok(())` if the spawn is permitted (or if there are no
    /// permissions to check), or `Err(SpawnError::PermissionDenied)` if
    /// the spawn is fully denied.
    async fn validate_permissions(
        &self,
        config: &ResolvedAgentConfig,
        parent_session_id: &str,
    ) -> Result<(), SpawnError> {
        let child_perms = self
            .config_manager
            .agent_permissions()
            .get(&config.id)
            .cloned();
        let parent_agent_id = self.session_manager.get_chat_id(parent_session_id).await;
        if let (Some(child_perms), Some(parent_agent_id)) = (child_perms, parent_agent_id) {
            let parent_perms = collect_chain_effective_permissions(
                &*self.session_manager,
                &self.config_manager.agent_permissions(),
                parent_session_id,
                &parent_agent_id,
            )
            .await;
            if let Some(parent_perms) = parent_perms {
                let user_id = self.session_manager.get_sender_id(parent_session_id).await;
                // Owner skips User dimension — design doc:
                // "Owner(User ID = 'owner') → skip User dim, only Agent"
                let user_perms = if user_id.as_deref() == Some("owner") {
                    None
                } else {
                    user_id.as_ref().map(|uid| {
                        self.permission_engine
                            .evaluate_user_permissions(uid, &config.id)
                    })
                };
                // Collect full-chain deny subjects from all ancestors.
                let rules = self.permission_engine.rules().clone();
                let chain_deny_subjects = collect_chain_deny_subjects(
                    &*self.session_manager,
                    &rules,
                    parent_session_id,
                    &config.id,
                )
                .await;
                let extra_deny = if chain_deny_subjects.is_empty() {
                    None
                } else {
                    Some(chain_deny_subjects.as_slice())
                };

                self.permission_engine
                    .validate_and_inject_spawn(
                        &config.id,
                        &child_perms,
                        &parent_perms,
                        user_perms.as_ref(),
                        user_id.as_deref(),
                        extra_deny,
                    )
                    .map_err(|e| {
                        tracing::debug!(error = %e, "spawn permission denied");
                        SpawnError::PermissionDenied {
                            agent_id: config.id.clone(),
                            reason: e.to_string(),
                        }
                    })?;
            }
        }
        Ok(())
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
        parent_agent_id: &str,
    ) -> Result<ResolvedParentDepth, SpawnError> {
        let parent_max_spawn_depth = {
            let agents = self.config_manager.agents();
            agents
                .get(parent_agent_id)
                .and_then(|pc| pc.subagents.max_spawn_depth)
                .unwrap_or(1u32)
        };

        let parent_effective_budget = self
            .session_manager
            .get_effective_max_spawn_depth(parent_session_id)
            .await
            .unwrap_or(parent_max_spawn_depth);

        if parent_effective_budget == 0 {
            return Err(SpawnError::DepthExceeded { current: 1, max: 0 });
        }

        Ok(ResolvedParentDepth {
            parent_effective_budget,
        })
    }

    /// Read parent agent config fields needed for concurrency/whitelist checks.
    /// Does NOT perform agentId fallback — that happens later in
    /// [`Self::resolve_target_config`].
    async fn read_parent_config(
        &self,
        parent_agent_id: &str,
    ) -> Result<ParentSpawnConfig, SpawnError> {
        let agents = self.config_manager.agents();

        Ok(match agents.get(parent_agent_id) {
            Some(pc) => {
                let sc = &pc.subagents;
                ParentSpawnConfig {
                    max_children: sc.max_children.unwrap_or(5),
                    allow_agents: sc.allow_agents.clone(),
                    require_agent_id: sc.require_agent_id.unwrap_or(false),
                }
            }
            None => ParentSpawnConfig {
                max_children: 5u32,
                allow_agents: vec!["*".to_string()],
                require_agent_id: false,
            },
        })
    }

    /// Resolve the target agent configuration under a single lock block.
    /// Handles the agentId fallback: if no `target_agent_id` is provided,
    /// falls back to the parent config's `default_child_agent`.
    async fn resolve_target_config(
        &self,
        parent_agent_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<ResolvedTarget, SpawnError> {
        let (target_id, target_config) = {
            let agents = self.config_manager.agents();
            let parent_config = agents.get(parent_agent_id);

            let target_id = target_agent_id
                .map(|s| s.to_string())
                .or_else(|| parent_config.and_then(|pc| pc.subagents.default_child_agent.clone()));

            let target_config = target_id.as_ref().and_then(|id| agents.get(id).cloned());

            (target_id, target_config)
        };

        Ok(ResolvedTarget {
            target_id,
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

// ═══════════════════════════════════════════════════════════════════════════
// SpawnValidator — bridge to closeclaw_config trait
// ═══════════════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
impl closeclaw_config::spawn_validation::SpawnValidator for SpawnController {
    async fn validate_spawn(
        &self,
        parent_session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<
        closeclaw_config::spawn_validation::SpawnValidationResult,
        closeclaw_config::spawn_validation::SpawnError,
    > {
        let result = self
            .validate(parent_session_id, target_agent_id)
            .await
            .map_err(|e| match e {
                SpawnError::DepthExceeded { current, max } => {
                    closeclaw_config::spawn_validation::SpawnError::DepthExceeded { current, max }
                }
                SpawnError::MaxChildrenReached { current, max } => {
                    closeclaw_config::spawn_validation::SpawnError::MaxChildrenReached {
                        current,
                        max,
                    }
                }
                SpawnError::AgentNotAllowed { agent_id } => {
                    closeclaw_config::spawn_validation::SpawnError::AgentNotAllowed { agent_id }
                }
                SpawnError::AgentIdRequired => {
                    closeclaw_config::spawn_validation::SpawnError::AgentIdRequired
                }
                SpawnError::ConfigNotFound(id) => {
                    closeclaw_config::spawn_validation::SpawnError::ConfigNotFound(id)
                }
                SpawnError::PermissionDenied { agent_id, reason } => {
                    closeclaw_config::spawn_validation::SpawnError::PermissionDenied {
                        agent_id,
                        reason,
                    }
                }
            })?;
        Ok(closeclaw_config::spawn_validation::SpawnValidationResult {
            config: result.config,
            effective_max_spawn_depth: result.effective_max_spawn_depth,
        })
    }
}
