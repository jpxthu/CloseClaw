//! Test-only trait adapters for cross-crate permission abstractions.
//!
//! Mirrors `crates/daemon/src/trait_adapters.rs`. The system-prompt crate
//! cannot depend on `closeclaw-daemon` (which depends on this crate), so
//! these adapters are duplicated here for tests that wire up the tool
//! registrars via `SessionToolsRegistrar`.

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

use closeclaw_common::agent_lookup::AgentLookup;
use closeclaw_common::permission_types::{
    ApprovalSubmission, CallerInfo, PermissionEvalResponse, PermissionEvaluator, RiskLevel,
};
use closeclaw_common::{BootstrapMode, ModelSpec};
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_risk::assess_risk_level;
use closeclaw_permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_permission::PermissionEngine;

/// Wrapper around `Arc<tokio::sync::RwLock<PermissionEngine>>` implementing
/// [`PermissionEvaluator`].
pub struct PermissionEngineAdapter(pub Arc<tokio::sync::RwLock<PermissionEngine>>);

#[async_trait]
impl PermissionEvaluator for PermissionEngineAdapter {
    async fn evaluate_inter_agent(&self, from: &str, to: &str) -> PermissionEvalResponse {
        let body = PermissionRequestBody::InterAgentMsg {
            from: from.to_string(),
            to: to.to_string(),
        };
        let engine = self.0.read().await;
        match engine.evaluate(PermissionRequest::Bare(body), None) {
            PermissionResponse::Allowed { .. } => PermissionEvalResponse::Allowed,
            PermissionResponse::Denied { reason, .. } => {
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
/// [`ApprovalSubmission`].
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

// ---------------------------------------------------------------------------
// Fake AgentLookup for SystemPromptBuilderAdapter tests
// ---------------------------------------------------------------------------

/// Fake [`AgentLookup`] backed by a fixed agent-ID → [`BootstrapMode`] map.
///
/// Lets adapter tests configure bootstrap modes without depending on the
/// `closeclaw-agent` registry.
#[derive(Default)]
pub struct FakeAgentLookup {
    bootstrap_modes: std::collections::HashMap<String, BootstrapMode>,
}

impl FakeAgentLookup {
    /// Create an empty fake — every query returns `None`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `mode` for `agent_id` (builder style).
    pub fn with_bootstrap_mode(mut self, agent_id: &str, mode: BootstrapMode) -> Self {
        self.bootstrap_modes.insert(agent_id.to_string(), mode);
        self
    }
}

#[async_trait]
impl AgentLookup for FakeAgentLookup {
    async fn get_agent_model(&self, _agent_id: &str) -> Option<ModelSpec> {
        None
    }

    async fn agent_exists(&self, agent_id: &str) -> bool {
        self.bootstrap_modes.contains_key(agent_id)
    }

    async fn query_bootstrap_mode(&self, agent_id: &str) -> Option<BootstrapMode> {
        self.bootstrap_modes.get(agent_id).copied()
    }

    async fn get_agent_workspace(&self, _agent_id: &str) -> Option<PathBuf> {
        None
    }
}
