//! Skill permission abstractions.
//!
//! Defines the traits and result type used by the skill layer to check
//! permissions and to submit denials into the approval flow. Implementations
//! live in [`crate::skill_wrapper`]; `RiskLevel` / `CallerInfo` are shared
//! types from `closeclaw-common`.

use async_trait::async_trait;
use closeclaw_common::permission_types::{CallerInfo, RiskLevel};

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
