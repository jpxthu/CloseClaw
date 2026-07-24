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

// ── Skill permission abstraction ─────────────────────────────────────────

/// Result of a skill-level permission evaluation.
#[derive(Debug, Clone)]
pub enum PermissionEvalResult {
    /// The operation is allowed.
    Allowed {
        /// Optional context modifier injected by the permission engine.
        context_modifier: Option<String>,
    },
    /// The operation is denied.
    Denied {
        /// Human-readable reason for the denial.
        reason: String,
        /// Risk level associated with the request.
        risk_level: RiskLevel,
    },
}

/// Trait for evaluating skill permission requests.
///
/// Skills use this trait to check whether an action on a resource is
/// permitted. The implementation (wrapper in `closeclaw-permission`)
/// translates `action`/`resource`/`details` into the engine's internal
/// `PermissionRequest` and returns a simplified result.
#[async_trait]
pub trait SkillPermissionChecker: Send + Sync {
    /// Check whether `action` on `resource` is permitted.
    ///
    /// `details` is a JSON object carrying action-specific context
    /// (e.g. `agent_id`, `path`, `cmd`, `host`, `port`).
    async fn check_permission(
        &self,
        action: &str,
        resource: &str,
        details: serde_json::Value,
    ) -> PermissionEvalResult;
}

/// Trait for submitting skill denial records to the approval flow.
///
/// When a permission check is denied and the caller supports approval,
/// the skill calls this trait to enqueue the denial for owner review.
#[async_trait]
pub trait SkillApprovalSubmitter: Send + Sync {
    /// Submit a denied request into the approval queue.
    ///
    /// Returns `Some(request_id)` if the denial was accepted into the
    /// approval queue, or `None` if rejected (e.g. sub-agent or duplicate).
    async fn submit_denial(
        &self,
        action: &str,
        resource: &str,
        reason: &str,
        risk_level: RiskLevel,
        session_id: &str,
        caller: &CallerInfo,
    ) -> Option<String>;
}

/// Type alias for shared skill permission checker reference.
pub type SharedSkillPermissionChecker = Arc<dyn SkillPermissionChecker>;

/// Type alias for shared skill approval submitter reference.
pub type SharedSkillApprovalSubmitter = Arc<dyn SkillApprovalSubmitter>;
