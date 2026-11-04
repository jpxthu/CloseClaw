//! Permission decision and approval payload families for cross-crate use.
//!
//! Defines the input/output payloads of one permission evaluation —
//! [`PermissionRequest`], [`PermissionOperation`], [`ExecutionPolicy`] and
//! [`PermissionDecision`] — the approval push and decision payloads —
//! [`ApprovalCard`], [`ApprovalDecision`], [`RegistrationConfirmation`] and
//! [`ApprovalDelivery`] — plus the auxiliary enums [`ExecLocation`],
//! [`MessageDirection`], [`ApprovalKind`], [`ApprovalDecisionKind`],
//! [`DecisionScope`] and [`RegistrationDecision`].
//!
//! Pure data carriers only: rule evaluation, approval queuing and audit
//! persistence belong to the `closeclaw-permission` module.

use serde::{Deserialize, Serialize};

use crate::permission_types::RiskLevel;

// ---------------------------------------------------------------------------
// Auxiliary enums
// ---------------------------------------------------------------------------

/// Execution location of a command (or of an [`ExecutionPolicy`]).
///
/// `Sandbox` is the default: an absent location is resolved by the sandbox
/// mode in effect.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecLocation {
    /// Execute inside the sandbox (no network capability).
    #[default]
    Sandbox,
    /// Execute on the host (bound to the policy's allowed paths/network).
    Host,
}

/// Direction of a user message (inbound from the User or outbound to it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    /// Message received from the User.
    Inbound,
    /// Message sent to the User.
    Outbound,
}

// ---------------------------------------------------------------------------
// Operation
// ---------------------------------------------------------------------------

/// One operation to be evaluated for permission.
///
/// Each variant carries the parameters needed for the evaluation and maps
/// one-to-one to the seven permission dimensions of the permission module.
/// Decision logic lives in the permission module; this type only carries
/// the input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOperation {
    /// Read a file at `path`.
    FileRead { path: String },
    /// Write a file at `path`.
    FileWrite { path: String },
    /// Run `command` at the given [`ExecLocation`] (default resolved by the
    /// sandbox mode in effect).
    Command {
        command: String,
        location: ExecLocation,
    },
    /// Access `host` on `port`.
    Network { host: String, port: u16 },
    /// Invoke the tool named `tool_name`.
    ToolCall { tool_name: String },
    /// Deliver a user message in the given [`MessageDirection`].
    UserMessage { direction: MessageDirection },
    /// Write configuration at `path`.
    ConfigWrite { path: String },
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// Input of one permission evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    /// Agent initiating the operation.
    pub agent_id: String,
    /// User ID of this round's caller — the sender of the inbound message
    /// that triggered this execution (not the session owner or the sender
    /// persisted in checkpoints).
    pub user_id: String,
    /// Whether the initiator is a sub-session. When true, denials are
    /// returned silently and never enter the approval queue.
    pub is_sub_session: bool,
    /// Session the operation belongs to, for approval-card attribution and
    /// audit lookup; empty when there is no session context.
    pub session_id: Option<String>,
    /// The operation to evaluate.
    pub operation: PermissionOperation,
    /// Purpose statement provided by the Agent for command-like operations,
    /// rendered on the approval card.
    pub purpose: Option<String>,
}

// ---------------------------------------------------------------------------
// Execution policy
// ---------------------------------------------------------------------------

/// Execution policy carried by an Allow conclusion.
///
/// Consumed by [`SandboxExecutor`](crate::SandboxExecutor) so execution
/// strictly follows the same evaluation — no re-evaluation or boundary
/// relaxation at execution time.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    /// Where to execute: sandbox or host.
    pub location: ExecLocation,
    /// Allowed read paths for sandboxed execution.
    pub read_paths: Vec<String>,
    /// Allowed write paths for sandboxed execution (empty for read-only).
    pub write_paths: Vec<String>,
    /// Paths denied at execution time (merged read/write deny paths; a
    /// single allow may remove one entry from this set).
    pub deny_paths: Vec<String>,
    /// Network ranges (host:port entries) allowed when executing on the
    /// host; empty means no network. Sandbox execution never has network.
    pub network_targets: Vec<String>,
}

// ---------------------------------------------------------------------------
// Decision
// ---------------------------------------------------------------------------

/// Conclusion of one permission evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Allowed, optionally carrying the [`ExecutionPolicy`] to execute with.
    Allowed {
        /// Execution policy for the allowed operation; `None` when the
        /// operation needs no execution boundary.
        policy: Option<ExecutionPolicy>,
    },
    /// Denied. Owner-approval waits happen inside the caller by the
    /// implementation, not here.
    Denied {
        /// Why the operation was denied — describes the denial only, never
        /// internal rule names, priorities or config paths.
        reason: String,
        /// Risk level of the denied operation (for audit and card hints).
        risk_level: RiskLevel,
    },
}

// ---------------------------------------------------------------------------
// Approval kinds
// ---------------------------------------------------------------------------

/// Kind of an approval request, rendered as the card type on the
/// [`ApprovalCard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    /// Permission denial waiting for the Owner's decision.
    Operation,
    /// Non-whitelisted command requesting execution on the host.
    HostExecution,
    /// Suspicious command flagged by the command security parser.
    Security,
    /// New user registration request.
    UserRegistration,
}

/// One decision option the Owner may pick for an operation approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionKind {
    /// Allow this operation once.
    Once,
    /// Add the operation object to the whitelist.
    Whitelist,
    /// Deny the operation.
    Deny,
}

/// Dimension an operation approval decision applies to.
///
/// Command-like and tool-call operations are fixed to
/// [`DecisionScope::Agent`]; when the Owner's own invocation is intercepted,
/// only [`DecisionScope::Agent`] is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionScope {
    /// Decision applies to the requesting Agent.
    Agent,
    /// Decision applies to the calling User.
    User,
    /// Decision applies to both the Agent and the User.
    Both,
}

/// Owner's confirmation decision for a new user registration request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationDecision {
    /// Approve the registration request.
    Approve,
    /// Reject the registration request.
    Reject,
}

// ---------------------------------------------------------------------------
// Approval card
// ---------------------------------------------------------------------------

/// Approval card pushed to the Owner (approval request payload).
///
/// Operation approvals are addressed by `approval_no`; new user registration
/// requests leave the operation fields empty and are addressed by
/// `request_id`. The two identifier spaces never overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalCard {
    /// Approval number — globally unique, increasing across sessions and
    /// channels; the Owner's approval commands target requests by this
    /// number. Empty for new user registration requests.
    pub approval_no: Option<u64>,
    /// Queue correlation identifier of a new user registration request
    /// (filled only when `kind` is [`ApprovalKind::UserRegistration`]);
    /// approval commands never address registration requests.
    pub request_id: Option<String>,
    /// Type of the approval request.
    pub kind: ApprovalKind,
    /// Agent initiating the operation; empty for registration requests.
    pub agent_id: Option<String>,
    /// User ID of this round's caller; empty for registration requests.
    pub user_id: Option<String>,
    /// Operation being evaluated with its parameters; empty for
    /// registration requests.
    pub operation: Option<PermissionOperation>,
    /// Risk level hint (for card display only, never decides whether an
    /// operation enters approval); registration requests use the default
    /// low level.
    pub risk_level: RiskLevel,
    /// Purpose statement provided for command-like operations.
    pub purpose: Option<String>,
    /// Parse failure reason and risk tags when the command security parser
    /// is uncertain about a command.
    pub security_note: Option<String>,
    /// Network access range (host:port entries) required by host execution
    /// requests.
    pub network_targets: Option<Vec<String>>,
    /// Decisions offered for this approval — high-risk operations and
    /// non-whitelistable objects (e.g. interpreter commands) exclude the
    /// whitelist option; registration requests carry an empty set (their
    /// decision form is [`RegistrationConfirmation`]).
    pub allowed_decisions: Vec<ApprovalDecisionKind>,
    /// Usage hint text — the approval command syntax (with the approval
    /// number and decision parameters) for operation approvals, or the
    /// registration confirmation command syntax for registration requests.
    pub usage_hint: String,
    /// Timeout instant (ISO 8601); on timeout the request ends as denied.
    pub deadline: String,
}

/// Owner's decision on one operation approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// Approval number the decision targets.
    pub approval_no: u64,
    /// The chosen decision.
    pub decision: ApprovalDecisionKind,
    /// Dimension the decision applies to.
    pub scope: DecisionScope,
}

/// Owner's confirmation of a new user registration request.
///
/// Lives in a separate identifier space from [`ApprovalDecision`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationConfirmation {
    /// Queue correlation identifier of the registration request.
    pub request_id: String,
    /// The confirmation decision.
    pub decision: RegistrationDecision,
}

/// Delivery result of one approval push.
///
/// The pusher only logs this result: no retries and no deadline extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDelivery {
    /// The card reached the Owner.
    Delivered,
    /// The Owner has no available channel.
    NoChannel,
    /// Delivery failed.
    Failed,
}
