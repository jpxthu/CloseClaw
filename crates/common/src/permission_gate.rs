//! Dependency-injection traits for permission evaluation, sandboxed
//! execution and approval notification (core-traits §权限评估与审批).
//!
//! Contracts live here so consumers (tools, session, gateway) never depend
//! on `closeclaw-permission`: the permission engine and its approval queue
//! implement [`PermissionGate`], the sandbox engine implements
//! [`SandboxExecutor`], and the gateway implements [`ApprovalNotifier`].
//! Each trait ships an [`Arc`]-based shared handle alias
//! ([`SharedPermissionGate`], [`SharedSandboxExecutor`],
//! [`SharedApprovalNotifier`]) for cross-module injection: daemon wraps the
//! concrete implementations and injects the aliases.
//!
//! Only trait signatures live here: rule evaluation, approval queuing and
//! sandbox enforcement belong to `closeclaw-permission`; card rendering and
//! channel selection belong to `closeclaw-gateway`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::permission_admission::{
    ApprovalCard, ApprovalDelivery, ExecutionPolicy, PermissionDecision, PermissionRequest,
};

// ---------------------------------------------------------------------------
// PermissionGate
// ---------------------------------------------------------------------------

/// Universal permission decision interface, including approval waiting.
///
/// Implemented by the permission crate's engine and approval queue (wrapped
/// and injected by daemon); consumed by tools, session and gateway to decide
/// one operation by one Agent acting as one User, without depending on the
/// permission crate.
///
/// Contract:
/// - Single entry point: [`PermissionGate::evaluate`] receives the request
///   (caller + operation + purpose) and returns the decision
///   ([`PermissionDecision::Allowed`] with the optional [`ExecutionPolicy`],
///   or [`PermissionDecision::Denied`] with reason and risk level). New
///   operation dimensions only extend
///   [`PermissionOperation`](crate::PermissionOperation), never this trait.
/// - Approval waiting completes inside the call: when the decision needs the
///   Owner's approval, the implementation waits within this call until the
///   Owner decides, the deadline elapses or the system interrupts. Callers
///   neither poll nor register callbacks; they may start the evaluation
///   asynchronously, report a pending-approval outcome to the Agent and
///   continue other work while waiting, and inject the conclusion into that
///   conversation round once it arrives.
/// - Audit: dispositions of approval-needing operations are persisted by the
///   implementation through the audit log; rule-level allow/deny only writes
///   debug logs. Callers never write audit entries themselves.
#[async_trait]
pub trait PermissionGate: Send + Sync {
    /// Evaluate one permission request and wait for the conclusion.
    ///
    /// Owner-approval waiting happens inside this call (see the trait
    /// contract): no polling, no callbacks; the returned decision is final
    /// for this request.
    async fn evaluate(&self, request: PermissionRequest) -> PermissionDecision;
}

// ---------------------------------------------------------------------------
// SandboxExecutor
// ---------------------------------------------------------------------------

/// Restricted-execution interface for sandboxed file access and commands.
///
/// Implemented by the permission crate's sandbox engine (OS-level isolation
/// and engine selection); consumed by tools' file tools and command
/// execution, so a released file access or command execution can be handed
/// to the sandbox without depending on the permission crate.
///
/// Contract:
/// - Policy binding: the [`ExecutionPolicy`] argument must be the one
///   produced by the same permission evaluation (taken from
///   [`PermissionDecision::Allowed`]) — execution never re-evaluates and
///   never widens the boundary. Execution on the host
///   ([`ExecLocation::Host`](crate::ExecLocation::Host)) does not go through
///   this interface at all.
/// - File access: the operation body is provided by the consumer and runs
///   inside the restricted execution environment under `policy`'s boundary.
///   Out-of-boundary access is rejected directly by the OS boundary
///   (surfaced as an I/O error). Released-mode file access is not
///   sandbox-enforced and never reaches this interface.
/// - Command execution: the command and its subprocesses run under
///   `policy`'s boundary; execution parameters such as the timeout are
///   provided by the caller at this entry. Processes inside the sandbox
///   never have network capability.
/// - No degradation: when the sandbox engine is unavailable, no execution
///   policy is produced ([`PermissionGate`] terminally rejects sandbox-bound
///   operations); this interface offers no fallback and no host-execution
///   path.
#[async_trait]
pub trait SandboxExecutor: Send + Sync {
    /// Execute one file operation inside the boundary of `policy`.
    ///
    /// The body is supplied by the consumer as a boxed closure capturing its
    /// own inputs; read results are stored into the body's captured state,
    /// keeping the signature free of consumer-specific result types.
    ///
    /// Implementor requirement: the body is synchronous; run it outside the
    /// async runtime (e.g. on a blocking thread pool via `spawn_blocking`)
    /// — implementations must never block an async context (workspace rule:
    /// no blocking inside async contexts, CONTRIBUTING.md §异步代码).
    async fn execute_file_operation(
        &self,
        policy: &ExecutionPolicy,
        operation: Box<dyn FnOnce() -> std::io::Result<()> + Send>,
    ) -> std::io::Result<()>;

    /// Execute `command` and its subprocesses inside the boundary of
    /// `policy` and return the process result (status, stdout, stderr).
    async fn execute_command(
        &self,
        policy: &ExecutionPolicy,
        command: &str,
        timeout: Duration,
    ) -> std::io::Result<std::process::Output>;
}

// ---------------------------------------------------------------------------
// ApprovalNotifier
// ---------------------------------------------------------------------------

/// Approval notification delivery interface.
///
/// Implemented by gateway (renders platform cards via the IM adapter and
/// delivers to the Owner's most recently used IM platform with fallback; the
/// CLI channel prints terminal hints); consumed by the permission crate's
/// approval queue to push approval requests and learn the delivery result,
/// so permission never depends on gateway.
///
/// Contract:
/// - No retry: the delivery result is only debug-logged by the permission
///   side; this interface never retries and never extends the approval wait
///   (the wait timer starts at request creation).
/// - Full refresh: [`ApprovalNotifier::refresh`] carries all currently
///   pending approval cards so the CLI channel can render one terminal hint
///   covering every waiting request in a single pass (IM channels deliver
///   per-card increments).
#[async_trait]
pub trait ApprovalNotifier: Send + Sync {
    /// Deliver one approval card to the Owner's available channel (IM
    /// platform card or CLI terminal hint) and return the delivery result.
    async fn deliver(&self, card: ApprovalCard) -> ApprovalDelivery;

    /// Refresh delivery with the full list of pending approval cards.
    async fn refresh(&self, cards: Vec<ApprovalCard>);
}

// ---------------------------------------------------------------------------
// Shared handle aliases
// ---------------------------------------------------------------------------

/// Shared handle alias for [`PermissionGate`].
///
/// The `Arc` wrapper only guards handle access, never the evaluation wait:
/// no lock is held while an approval is pending inside
/// [`PermissionGate::evaluate`].
pub type SharedPermissionGate = Arc<dyn PermissionGate + Send + Sync>;

/// Shared handle alias for [`SandboxExecutor`].
pub type SharedSandboxExecutor = Arc<dyn SandboxExecutor + Send + Sync>;

/// Shared handle alias for [`ApprovalNotifier`].
pub type SharedApprovalNotifier = Arc<dyn ApprovalNotifier + Send + Sync>;
