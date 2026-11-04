//! Permission decision payload family for cross-crate use.
//!
//! Defines the input/output payloads of one permission evaluation —
//! [`PermissionRequest`], [`PermissionOperation`], [`ExecutionPolicy`] and
//! [`PermissionDecision`] — plus the two auxiliary enums [`ExecLocation`]
//! and [`MessageDirection`].
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
/// Consumed by [`SandboxExecutor`](crate) so execution strictly follows the
/// same evaluation — no re-evaluation or boundary relaxation at execution
/// time.
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
