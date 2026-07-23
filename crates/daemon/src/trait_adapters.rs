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
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_risk::assess_risk_level;
use closeclaw_permission::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody,
};
use closeclaw_permission::PermissionEngine;

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
            creator_id: caller.creator_id.clone(),
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
