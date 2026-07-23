//! Permission trait abstractions for cross-crate use.
//!
//! Defines minimal traits and types for permission evaluation and approval
//! submission that can be used by crates that cannot directly depend on
//! `closeclaw-permission` (e.g., `closeclaw-session`).

use async_trait::async_trait;
use std::sync::Arc;

/// Risk level for permission requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Simplified inter-agent message body for permission evaluation.
#[derive(Debug, Clone)]
pub struct InterAgentMsg {
    pub from: String,
    pub to: String,
}

/// Result of a permission evaluation.
#[derive(Debug, Clone)]
pub enum PermissionEvalResponse {
    Allowed,
    Denied {
        reason: String,
        risk_level: RiskLevel,
    },
}

/// Caller information for approval submission.
#[derive(Debug, Clone)]
pub struct CallerInfo {
    pub user_id: String,
    pub agent: String,
    pub creator_id: String,
}

/// Trait for evaluating inter-agent permission requests.
///
/// Implemented by the permission crate's `PermissionEngine` (via wrapper).
/// Session tools depend on this trait to avoid a direct dependency on
/// `closeclaw-permission`.
#[async_trait]
pub trait PermissionEvaluator: Send + Sync {
    /// Evaluate an inter-agent message permission request.
    async fn evaluate_inter_agent(&self, from: &str, to: &str) -> PermissionEvalResponse;
}

/// Trait for submitting permission denials to the approval flow.
///
/// Implemented by the permission crate's `ApprovalFlow` (via wrapper).
/// Session tools depend on this trait to avoid a direct dependency on
/// `closeclaw-permission`.
pub trait ApprovalSubmission: Send + Sync {
    /// Submit a denied inter-agent request for owner approval.
    ///
    /// Returns `Some(request_id)` if the denial was accepted into the
    /// approval queue, or `None` if rejected (e.g., sub-agent or duplicate).
    fn submit_inter_agent_denial(
        &self,
        caller: &CallerInfo,
        from: &str,
        to: &str,
        risk_level: RiskLevel,
        session_id: &str,
        is_sub_agent: bool,
    ) -> Option<String>;
}

/// Type alias for shared permission evaluator reference.
pub type SharedPermissionEvaluator = Arc<dyn PermissionEvaluator>;

/// Type alias for shared approval submission reference.
pub type SharedApprovalSubmission = Arc<tokio::sync::Mutex<dyn ApprovalSubmission>>;
