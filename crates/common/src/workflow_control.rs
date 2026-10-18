//! Trait for workflow runtime control invoked by the gateway.
//!
//! Abstracts the Workflow Engine (in the workflow crate) so that the gateway
//! can start workflows, forward owner messages, and terminate workflows on
//! `/stop` without depending on the workflow crate directly. This breaks the
//! gateway → workflow dependency while keeping the engine free to evolve
//! behind a stable, type-erased boundary (same precedent as
//! [`SlashSessionQuery`](crate::SlashSessionQuery)).

use async_trait::async_trait;

/// Workflow runtime control interface for the gateway.
///
/// Implemented by the Workflow Engine in the workflow crate; the gateway
/// depends only on this trait (defined in common). Signal semantics follow
/// `docs/design/common/core-traits.md#workflowcontrol` and
/// `docs/design/workflow/workflow-tools.md` (WorkflowControl 信号).
#[async_trait]
pub trait WorkflowControl: Send + Sync {
    /// Start a workflow for a session.
    ///
    /// Confirms no active `WorkflowRun` exists, loads and validates the
    /// workflow definition by `workflow_name`, initializes the run, and
    /// injects the workflow context plus the Step 0 goal message. Returns
    /// an error on failure. Invoked by the gateway to execute
    /// `SlashResult::WorkflowStart`.
    async fn start(&self, session_id: &str, workflow_name: &str) -> Result<(), String>;

    /// Notify the engine of an owner (non-slash) message before routing.
    ///
    /// Called by the gateway before an owner non-slash message is routed to
    /// the session. If the session has a resumable pause (blocked for reasons
    /// other than a definition version change), the block is lifted;
    /// a terminal pause (definition version change) is a no-op, as is any
    /// other phase.
    async fn on_owner_message(&self, session_id: &str) -> Result<(), String>;

    /// Terminate the active workflow of a session.
    ///
    /// Called by the gateway after the owner's `/stop` has completed the
    /// forced stop. Terminates the session's active workflow and runs exit
    /// cleanup (remove workflow context, clear control messages, clear and
    /// persist `WorkflowRun`). A no-op when no workflow is active (idempotent).
    async fn terminate(&self, session_id: &str) -> Result<(), String>;
}
