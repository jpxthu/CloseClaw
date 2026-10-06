//! SpawnController — validates spawn requests against agent configuration.
//!
//! This module defines the `SpawnController` struct and the trait-based
//! dependency injection (`SpawnContext`, `SpawnBudgetLookup`,
//! `PermissionChecker`) used to validate spawn requests without
//! depending on the gateway, config, or permission crates directly.
//!
//! The controller validates depth, concurrency, whitelist, and permission
//! checks before allowing a spawn.

use std::path::PathBuf;
use std::sync::Arc;

use closeclaw_common::{
    HookConfig, ModelSpec, PermissionChecker, SpawnError, SpawnValidationResult, SpawnValidator,
};

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

/// Raw spawn-budget fields for one agent, mapped from its config's
/// `subagents` block.
///
/// `Option` fields mirror the config verbatim — defaulting stays inside
/// [`SpawnController`], so [`SpawnBudgetLookup`] implementations remain
/// a pure data copy.
#[derive(Debug, Clone, Default)]
pub struct AgentSpawnBudget {
    /// Maximum nested spawn depth (`subagents.maxSpawnDepth`).
    pub max_spawn_depth: Option<u32>,
    /// Maximum concurrent active children (`subagents.maxChildren`).
    pub max_children: Option<u32>,
    /// Target agent whitelist (`subagents.allowAgents`).
    pub allow_agents: Option<Vec<String>>,
    /// Whether agentId must be explicit (`subagents.requireAgentId`).
    pub require_agent_id: Option<bool>,
    /// Sub-agent execution timeout in seconds (`subagents.timeout`).
    pub timeout: Option<u64>,
    /// Cyclic warning start offset in seconds (`subagents.timeoutWarning`).
    pub timeout_warning: Option<u64>,
    /// Cyclic warning interval ratio
    /// (`subagents.timeoutNotifyIntervalRatio`).
    pub timeout_notify_interval_ratio: Option<f64>,
}

/// Narrow spawn-time view of the target agent's configuration.
///
/// Carries only the fields consumed by the child-session creation chain
/// (identity, model, workspace, skills, tools, hooks) using common
/// types — the full config profile never enters the shared structure
/// (design doc §shared-types SpawnValidationResult).
#[derive(Debug, Clone, Default)]
pub struct SpawnTargetAgentConfig {
    /// Agent identifier.
    pub id: String,
    /// Configured model, if any.
    pub model: Option<ModelSpec>,
    /// Per-agent workspace path, if configured.
    pub workspace: Option<PathBuf>,
    /// Agent skills whitelist.
    pub skills: Vec<String>,
    /// Agent tools whitelist.
    pub tools: Vec<String>,
    /// Run-health hook configuration.
    pub hooks: Vec<HookConfig>,
}

/// Narrow query channel for spawn budget and target-agent data.
///
/// Session-owned port: the composition root (daemon) implements it over
/// the config store and injects it into [`SpawnController`], so the
/// controller reads budget fields without depending on the config crate.
#[async_trait::async_trait]
pub trait SpawnBudgetLookup: Send + Sync {
    /// Look up one agent's raw spawn-budget fields; `None` if unknown.
    async fn spawn_budget(&self, agent_id: &str) -> Option<AgentSpawnBudget>;

    /// Look up the narrow spawn-time config view; `None` if unknown.
    async fn spawn_target_config(&self, agent_id: &str) -> Option<SpawnTargetAgentConfig>;
}

/// Session-internal channel for resolving a target agent's spawn-time
/// config view.
///
/// [`SpawnValidationResult`] carries only the target `agent_id` plus its
/// derived parameters — the full config profile never enters the shared
/// structure (design doc §shared-types SpawnValidationResult). Consumers
/// inside the session domain (`SessionsSpawnTool`) obtain the target
/// config view through this channel instead.
///
/// Implemented by [`SpawnController`]; injected by the session tools
/// registrar (composition root assembles it).
#[async_trait::async_trait]
pub trait SpawnTargetConfigLookup: Send + Sync {
    /// Resolve the spawn-time config view for `agent_id`; `None` if unknown.
    async fn resolve_agent_config(&self, agent_id: &str) -> Option<SpawnTargetAgentConfig>;
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

/// Result from resolving target agent budget (agentId fallback).
struct ResolvedTarget {
    target_id: Option<String>,
    target_budget: Option<AgentSpawnBudget>,
}

/// Validates spawn requests and computes effective child configuration.
///
/// Uses trait-based dependency injection (`SpawnContext` for session
/// queries, `SpawnBudgetLookup` for agent budget fields, and
/// `PermissionChecker` for permission checks) to stay decoupled from the
/// gateway, config, and permission crates.
pub struct SpawnController {
    budget_lookup: Arc<dyn SpawnBudgetLookup>,
    context: Arc<dyn SpawnContext>,
    permission_checker: Arc<dyn PermissionChecker>,
}

// ── Constructor + Validation API ──────────────────────────────────────

/// Global default spawn timeout in seconds (48 hours).
/// This is the final fallback when neither spawn args nor target agent
/// configuration specify a timeout.
const DEFAULT_SPAWN_TIMEOUT_SECS: u64 = 172800;

impl SpawnController {
    pub fn new(
        budget_lookup: Arc<dyn SpawnBudgetLookup>,
        context: Arc<dyn SpawnContext>,
        permission_checker: Arc<dyn PermissionChecker>,
    ) -> Self {
        Self {
            budget_lookup,
            context,
            permission_checker,
        }
    }

    /// Validate a spawn request.
    ///
    /// Returns a [`SpawnValidationResult`] with the target agent identifier
    /// and the effective max spawn depth for the child, or a
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
        let parent_cfg = self.read_parent_config(&parent_agent_id).await;
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
        let resolved = self
            .resolve_target_budget(&parent_agent_id, target_agent_id)
            .await?;
        let target_id = resolved.target_id.ok_or(SpawnError::AgentIdRequired)?;

        // ⑤ Whitelist check (design doc §Spawn 控制流程 ⑤):
        //    Reject if target agent not in allowAgents.
        self.check_whitelist(&target_id, &parent_cfg.allow_agents)?;

        // Permission check is now a separate step — tools layer calls
        // check_spawn_permission() after validate() succeeds.
        // (design doc §Spawn 控制流程 — two-step separation)
        let budget = resolved
            .target_budget
            .ok_or_else(|| SpawnError::ConfigNotFound(target_id.clone()))?;

        // Compute effective_max_spawn_depth and validate child depth.
        let effective_max =
            self.compute_effective_max_depth(parent.parent_effective_budget, &budget)?;
        let spawn_timeout = self.resolve_spawn_timeout(&budget);
        let (timeout_warning_secs, timeout_notify_interval_ratio) =
            self.resolve_timeout_warning(&budget);

        // Only the derived parameters + target agent id enter the shared
        // structure — the full config stays inside the session domain
        // (design doc §shared-types SpawnValidationResult).
        Ok(SpawnValidationResult {
            agent_id: target_id,
            effective_max_spawn_depth: effective_max,
            spawn_timeout,
            timeout_warning_secs,
            timeout_notify_interval_ratio,
        })
    }

    /// Check spawn permissions after preconditions have passed.
    ///
    /// This is the second step of the two-step spawn validation
    /// architecture (design doc §Spawn 控制流程 — two-step separation).
    /// The full target config is re-resolved internally from the
    /// validation's `agent_id` — it never leaves the session domain.
    pub async fn check_spawn_permission(
        &self,
        parent_session_id: &str,
        validation: &SpawnValidationResult,
    ) -> Result<(), SpawnError> {
        // Re-check target agent existence before the permission verdict:
        // an unknown agent fails with ConfigNotFound (two-step separation).
        if self
            .budget_lookup
            .spawn_budget(&validation.agent_id)
            .await
            .is_none()
        {
            return Err(SpawnError::ConfigNotFound(validation.agent_id.clone()));
        }
        self.validate_permissions(&validation.agent_id, parent_session_id)
            .await
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
        let parent_max_spawn_depth = self
            .budget_lookup
            .spawn_budget(parent_agent_id)
            .await
            .and_then(|b| b.max_spawn_depth)
            .unwrap_or(1u32);

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

    /// Read parent agent budget fields needed for concurrency/whitelist
    /// checks. Does NOT perform agentId fallback — that happens later in
    /// [`Self::resolve_target_budget`].
    async fn read_parent_config(&self, parent_agent_id: &str) -> ParentSpawnConfig {
        match self.budget_lookup.spawn_budget(parent_agent_id).await {
            Some(b) => ParentSpawnConfig {
                max_children: b.max_children.unwrap_or(5),
                allow_agents: b.allow_agents.unwrap_or_else(|| vec!["*".to_string()]),
                require_agent_id: b.require_agent_id.unwrap_or(false),
            },
            None => ParentSpawnConfig {
                max_children: 5u32,
                allow_agents: vec!["*".to_string()],
                require_agent_id: false,
            },
        }
    }

    /// Resolve the target agent budget under a single lookup.
    /// Handles the agentId fallback chain (design doc §Spawn 控制流程 ④):
    ///   1. Explicit `target_agent_id` (caller-provided)
    ///   2. Parent agent ID itself (spawn self-copy)
    async fn resolve_target_budget(
        &self,
        parent_agent_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<ResolvedTarget, SpawnError> {
        // Fallback chain: explicit → parent agent ID
        let target_id = target_agent_id
            .map(|s| s.to_string())
            .or_else(|| Some(parent_agent_id.to_string()));

        let target_budget = match &target_id {
            Some(id) => self.budget_lookup.spawn_budget(id).await,
            None => None,
        };

        Ok(ResolvedTarget {
            target_id,
            target_budget,
        })
    }
}

// ── Validation Helpers ────────────────────────────────────────────────

impl SpawnController {
    /// Compute the effective maximum spawn depth for a child session.
    fn compute_effective_max_depth(
        &self,
        parent_effective_budget: u32,
        target_budget: &AgentSpawnBudget,
    ) -> Result<u32, SpawnError> {
        let child_max_depth = target_budget.max_spawn_depth.unwrap_or(1);
        let effective_max = child_max_depth.min(parent_effective_budget.saturating_sub(1));
        Ok(effective_max)
    }

    /// Validate spawn permissions via the injected `PermissionChecker`.
    async fn validate_permissions(
        &self,
        agent_id: &str,
        parent_session_id: &str,
    ) -> Result<(), SpawnError> {
        self.permission_checker
            .validate_spawn_permission(agent_id, parent_session_id)
            .await
            .map_err(SpawnError::Permission)
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

    /// Resolve the spawn timeout using the priority chain:
    /// target agent's `subagents.timeout` → global default.
    ///
    /// Note: spawn args timeout is applied later in `SessionsSpawnTool::call()`
    /// after validation, as it takes highest priority in the chain.
    fn resolve_spawn_timeout(&self, target_budget: &AgentSpawnBudget) -> Option<u64> {
        target_budget
            .timeout
            .or_else(|| self.global_spawn_timeout())
    }

    /// Global default spawn timeout (seconds).
    ///
    /// This is the final fallback in the timeout priority chain:
    /// spawn args → target agent config → **global default**.
    fn global_spawn_timeout(&self) -> Option<u64> {
        Some(DEFAULT_SPAWN_TIMEOUT_SECS)
    }

    /// Resolve timeout_warning using the priority chain:
    /// target agent's `subagents.timeout_warning` → global default (None = legacy).
    fn resolve_timeout_warning(
        &self,
        target_budget: &AgentSpawnBudget,
    ) -> (Option<u64>, Option<f64>) {
        (
            target_budget.timeout_warning,
            target_budget.timeout_notify_interval_ratio,
        )
    }
}

// ── Trait impls ────────────────────────────────────────────────────

#[async_trait::async_trait]
impl SpawnTargetConfigLookup for SpawnController {
    /// Resolve the target agent's spawn-time config view from the
    /// injected lookup — only creation-chain fields are carried; the
    /// full profile never enters shared structures (session-internal
    /// channel).
    async fn resolve_agent_config(&self, agent_id: &str) -> Option<SpawnTargetAgentConfig> {
        self.budget_lookup.spawn_target_config(agent_id).await
    }
}

#[async_trait::async_trait]
impl SpawnValidator for SpawnController {
    async fn validate_spawn(
        &self,
        parent_session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<SpawnValidationResult, SpawnError> {
        // Both sides use the same SpawnValidationResult type after unification;
        // pass through directly without field-by-field copy.
        self.validate(parent_session_id, target_agent_id).await
    }

    async fn check_spawn_permission(
        &self,
        parent_session_id: &str,
        validation: &SpawnValidationResult,
    ) -> Result<(), SpawnError> {
        // Both sides use the same SpawnValidationResult type after unification;
        // pass through directly without reconstructing an identical struct.
        self.check_spawn_permission(parent_session_id, validation)
            .await
    }
}
