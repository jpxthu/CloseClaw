//! Trait adapters for cross-crate abstractions.
//!
//! Provides implementations of [`PermissionEvaluator`] and
//! [`ApprovalSubmission`] from `closeclaw_common` for the permission
//! crate's concrete types (`PermissionEngine` and `ApprovalFlow`).
//! These adapters allow session tools to use the abstracted traits
//! via `SessionToolsRegistrar` without depending on `closeclaw-permission`.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_common::permission_types::{
    ApprovalSubmission, CallerInfo, PermissionEvalResponse, PermissionEvaluator, RiskLevel,
};
use closeclaw_config::ConfigManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_risk::assess_risk_level;
use closeclaw_permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody,
};
use closeclaw_permission::PermissionEngine;
use closeclaw_session::spawn::controller::{
    AgentSpawnBudget, SpawnBudgetLookup, SpawnTargetAgentConfig,
};

/// Wrapper around `Arc<tokio::sync::RwLock<PermissionEngine>>` implementing
/// [`PermissionEvaluator`] so session tools can evaluate inter-agent
/// permissions without a direct dependency on `closeclaw-permission`.
pub struct PermissionEngineAdapter(pub Arc<tokio::sync::RwLock<PermissionEngine>>);

#[async_trait]
impl PermissionEvaluator for PermissionEngineAdapter {
    async fn evaluate_inter_agent(&self, from: &str, to: &str) -> PermissionEvalResponse {
        let body = PermissionRequestBody::InterAgentMsg {
            from: from.to_string(),
            to: to.to_string(),
        };
        let engine = self.0.read().await;
        let response = engine.evaluate(PermissionRequest::Bare(body), None);
        match response {
            closeclaw_permission::engine::engine_types::PermissionResponse::Allowed { .. } => {
                PermissionEvalResponse::Allowed
            }
            closeclaw_permission::engine::engine_types::PermissionResponse::Denied {
                reason,
                ..
            } => {
                let risk_level = assess_risk_level(&PermissionRequestBody::InterAgentMsg {
                    from: from.to_string(),
                    to: to.to_string(),
                });
                PermissionEvalResponse::Denied {
                    reason,
                    risk_level: map_risk_level(risk_level),
                }
            }
        }
    }
}

/// Wrapper around `Arc<tokio::sync::Mutex<ApprovalFlow>>` implementing
/// [`ApprovalSubmission`] so session tools can submit inter-agent
/// denials without a direct dependency on `closeclaw-permission`.
pub struct ApprovalFlowAdapter(pub Arc<tokio::sync::Mutex<ApprovalFlow>>);

impl ApprovalSubmission for ApprovalFlowAdapter {
    fn submit_inter_agent_denial(
        &self,
        caller: &CallerInfo,
        from: &str,
        to: &str,
        risk_level: RiskLevel,
        session_id: &str,
        is_sub_agent: bool,
    ) -> Option<String> {
        let permission_caller = Caller {
            user_id: caller.user_id.clone(),
            agent: caller.agent.clone(),
        };
        let body = PermissionRequestBody::InterAgentMsg {
            from: from.to_string(),
            to: to.to_string(),
        };
        let mut flow = self.0.blocking_lock();
        flow.submit_denial(
            &permission_caller,
            &body,
            map_risk_level_to_permission(risk_level),
            session_id,
            is_sub_agent,
        )
    }
}

/// Map permission crate's `RiskLevel` to common crate's `RiskLevel`.
fn map_risk_level(level: closeclaw_permission::engine::engine_risk::RiskLevel) -> RiskLevel {
    match level {
        closeclaw_permission::engine::engine_risk::RiskLevel::Low => RiskLevel::Low,
        closeclaw_permission::engine::engine_risk::RiskLevel::Medium => RiskLevel::Medium,
        closeclaw_permission::engine::engine_risk::RiskLevel::High => RiskLevel::High,
        closeclaw_permission::engine::engine_risk::RiskLevel::Critical => RiskLevel::Critical,
    }
}

/// Map common crate's `RiskLevel` to permission crate's `RiskLevel`.
fn map_risk_level_to_permission(
    level: RiskLevel,
) -> closeclaw_permission::engine::engine_risk::RiskLevel {
    match level {
        RiskLevel::Low => closeclaw_permission::engine::engine_risk::RiskLevel::Low,
        RiskLevel::Medium => closeclaw_permission::engine::engine_risk::RiskLevel::Medium,
        RiskLevel::High => closeclaw_permission::engine::engine_risk::RiskLevel::High,
        RiskLevel::Critical => closeclaw_permission::engine::engine_risk::RiskLevel::Critical,
    }
}

/// Config-backed production implementation of the session crate's
/// [`SpawnBudgetLookup`] port.
///
/// The daemon is the composition root owning the config store; this
/// adapter maps `ConfigManager` agent entries onto the narrow
/// spawn-budget / spawn-time config views. Pure data mapping — the
/// controller owns all defaulting and validation semantics. Injected
/// into `SpawnController` in `phase_wiring`.
pub struct ConfigSpawnBudgetLookup {
    config_manager: Arc<ConfigManager>,
}

impl ConfigSpawnBudgetLookup {
    pub fn new(config_manager: Arc<ConfigManager>) -> Self {
        Self { config_manager }
    }
}

#[async_trait]
impl SpawnBudgetLookup for ConfigSpawnBudgetLookup {
    async fn spawn_budget(&self, agent_id: &str) -> Option<AgentSpawnBudget> {
        let agents = self.config_manager.agents();
        let sc = &agents.get(agent_id)?.subagents;
        Some(AgentSpawnBudget {
            max_spawn_depth: sc.max_spawn_depth,
            max_children: sc.max_children,
            allow_agents: sc.allow_agents.clone(),
            require_agent_id: sc.require_agent_id,
            timeout: sc.timeout,
            timeout_warning: sc.timeout_warning,
            timeout_notify_interval_ratio: sc.timeout_notify_interval_ratio,
        })
    }

    async fn spawn_target_config(&self, agent_id: &str) -> Option<SpawnTargetAgentConfig> {
        let agents = self.config_manager.agents();
        let cfg = agents.get(agent_id)?;
        Some(spawn_target_agent_config(cfg))
    }
}

/// Map a resolved agent config onto the session-owned narrow
/// spawn-time view.
///
/// Pure data copy of the creation-chain fields — the full config
/// profile never crosses into the shared structure. Single-point
/// definition shared by [`ConfigSpawnBudgetLookup::spawn_target_config`]
/// and the phase-wiring child-session callback.
pub(super) fn spawn_target_agent_config(
    cfg: &closeclaw_config::agents::ResolvedAgentConfig,
) -> SpawnTargetAgentConfig {
    SpawnTargetAgentConfig {
        id: cfg.id.clone(),
        model: cfg.model.clone(),
        workspace: cfg.workspace.clone(),
        skills: cfg.skills.clone(),
        tools: cfg.tools.clone(),
        hooks: cfg.hooks.clone(),
    }
}
