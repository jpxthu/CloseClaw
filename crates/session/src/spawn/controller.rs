//! SpawnController — validates spawn requests against agent configuration.
//!
//! This module defines the `SpawnController` struct and the trait-based
//! dependency injection (`SpawnContext`, `PermissionChecker`) used to
//! validate spawn requests without depending on the gateway or permission
//! crates directly.
//!
//! The controller validates depth, concurrency, whitelist, and permission
//! checks before allowing a spawn.

use std::sync::Arc;

use closeclaw_common::{PermissionChecker, SpawnPermissionError};
use closeclaw_config::agents::ResolvedAgentConfig;

use super::error::SpawnError;
use super::types::SpawnValidationResult;

/// Dependency injection trait for querying active child session counts
/// and session metadata.
///
/// Implemented by `SessionManager` on the gateway side to decouple
/// `SpawnController` from the concrete SessionManager type.
#[async_trait::async_trait]
pub trait SpawnContext: Send + Sync {
    /// Return the number of active (non-completed) child sessions
    /// for the given parent session.
    async fn active_children_count(&self, parent_session_id: &str) -> usize;

    /// Look up the agent ID (chat_id) associated with a session.
    async fn chat_id(&self, session_id: &str) -> Option<String>;

    /// Look up the sender/user ID associated with a session.
    async fn sender_id(&self, session_id: &str) -> Option<String>;

    /// Get the effective max spawn depth budget for a session.
    async fn effective_max_spawn_depth(&self, session_id: &str) -> Option<u32>;
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
    timeout: Option<u64>,
}

/// Result from resolving target agent configuration (agentId fallback).
struct ResolvedTarget {
    target_id: Option<String>,
    target_config: Option<ResolvedAgentConfig>,
}

/// Validates spawn requests and computes effective child configuration.
///
/// Uses trait-based dependency injection (`SpawnContext` for session queries,
/// `PermissionChecker` for permission checks) to stay decoupled from the
/// gateway and permission crates.
pub struct SpawnController {
    config_manager: Arc<closeclaw_config::ConfigManager>,
    context: Arc<dyn SpawnContext>,
    permission_checker: Arc<dyn PermissionChecker>,
}

// ── Constructor + Validation API ──────────────────────────────────────

impl SpawnController {
    pub fn new(
        config_manager: Arc<closeclaw_config::ConfigManager>,
        context: Arc<dyn SpawnContext>,
        permission_checker: Arc<dyn PermissionChecker>,
    ) -> Self {
        Self {
            config_manager,
            context,
            permission_checker,
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
        // ① Depth check: reject if effective budget ≤ 0 (design doc §Spawn 控制流程 ①).
        let parent_agent_id = self
            .context
            .chat_id(parent_session_id)
            .await
            .unwrap_or_default();
        let parent = self
            .resolve_parent_depth(parent_session_id, &parent_agent_id)
            .await?;

        // ② Concurrency check: reject if active children ≥ maxChildren (design doc §Spawn 控制流程 ②).
        let parent_cfg = self.read_parent_config(&parent_agent_id);
        self.check_concurrency(parent_session_id, parent_cfg.max_children)
            .await?;

        // ③ requireAgentId check (design doc §Spawn 控制流程 ③):
        //    Must come before agentId resolution. When requireAgentId=true
        //    and no agentId provided, reject immediately without fallback
        //    or whitelist checks.
        if parent_cfg.require_agent_id && target_agent_id.is_none() {
            return Err(SpawnError::AgentIdRequired);
        }

        // ④ AgentId resolution (design doc §Spawn 控制流程 ④):
        //    Default to parent agent ID when not provided.
        let resolved = self.resolve_target_config(&parent_agent_id, target_agent_id)?;
        let target_id = resolved.target_id.ok_or(SpawnError::AgentIdRequired)?;

        // ⑤ Whitelist check (design doc §Spawn 控制流程 ⑤):
        //    Reject if target agent not in allowAgents.
        self.check_whitelist(&target_id, &parent_cfg.allow_agents)?;

        // ⑥ Permission check (design doc §Spawn 控制流程 ⑥):
        //    Validate child permissions via intersection with parent
        //    effective permissions.
        let config = resolved
            .target_config
            .ok_or(SpawnError::ConfigNotFound(target_id))?
            .clone();
        self.validate_permissions(&config, parent_session_id)
            .await?;

        // Compute effective_max_spawn_depth and validate child depth.
        let effective_max =
            self.compute_effective_max_depth(parent.parent_effective_budget, Some(&config))?;
        let spawn_timeout = self.read_spawn_timeout(&parent_agent_id);

        Ok(SpawnValidationResult {
            config,
            effective_max_spawn_depth: effective_max,
            spawn_timeout,
        })
    }
}

// ── Config Resolution ─────────────────────────────────────────────────

impl SpawnController {
    /// Resolve the parent session's depth and maximum spawn budget.
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
            .context
            .effective_max_spawn_depth(parent_session_id)
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
    fn read_parent_config(&self, parent_agent_id: &str) -> ParentSpawnConfig {
        let agents = self.config_manager.agents();

        match agents.get(parent_agent_id) {
            Some(pc) => {
                let sc = &pc.subagents;
                ParentSpawnConfig {
                    max_children: sc.max_children.unwrap_or(5),
                    allow_agents: sc.allow_agents.clone(),
                    require_agent_id: sc.require_agent_id.unwrap_or(false),
                    timeout: sc.timeout,
                }
            }
            None => ParentSpawnConfig {
                max_children: 5u32,
                allow_agents: vec!["*".to_string()],
                require_agent_id: false,
                timeout: None,
            },
        }
    }

    /// Resolve the target agent configuration under a single lock block.
    /// Handles the agentId fallback chain (design doc §Spawn 控制流程 ④):
    ///   1. Explicit `target_agent_id` (caller-provided)
    ///   2. Parent agent ID itself (spawn self-copy)
    fn resolve_target_config(
        &self,
        parent_agent_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<ResolvedTarget, SpawnError> {
        let agents = self.config_manager.agents();

        // Fallback chain: explicit → parent agent ID
        let target_id = target_agent_id
            .map(|s| s.to_string())
            .or_else(|| Some(parent_agent_id.to_string()));

        let target_config = target_id.as_ref().and_then(|id| agents.get(id).cloned());

        Ok(ResolvedTarget {
            target_id,
            target_config,
        })
    }
}

// ── Validation Helpers ────────────────────────────────────────────────

impl SpawnController {
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

    /// Validate spawn permissions via the injected `PermissionChecker`.
    async fn validate_permissions(
        &self,
        config: &ResolvedAgentConfig,
        parent_session_id: &str,
    ) -> Result<(), SpawnError> {
        self.permission_checker
            .validate_spawn_permission(&config.id, parent_session_id)
            .await
            .map_err(|e| match e {
                SpawnPermissionError::Denied { agent_id, reason } => {
                    SpawnError::PermissionDenied { agent_id, reason }
                }
            })
    }

    /// Check that the parent has not reached its maximum concurrent children.
    async fn check_concurrency(
        &self,
        parent_session_id: &str,
        max_children: u32,
    ) -> Result<(), SpawnError> {
        let active = self.context.active_children_count(parent_session_id).await;
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

    /// Read the parent agent's `subagents.timeout` value.
    fn read_spawn_timeout(&self, parent_agent_id: &str) -> Option<u64> {
        self.read_parent_config(parent_agent_id).timeout
    }
}
