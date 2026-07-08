//! Progress notification trait — decouples execution engine from session layer.
//!
//! The engine calls [`PlanStateNotifier::on_progress_changed`] whenever
//! a step status transition succeeds, passing the formatted progress
//! summary. Implementations decide how to present the information
//! (e.g. inject into system prompt, log, send to UI).

use async_trait::async_trait;

/// Callback interface for plan progress changes.
///
/// Defined in `closeclaw-common` (Layer 0) so both the execution engine
/// (Layer 3) and the session layer (Layer 2) can depend on it without
/// creating a cross-layer coupling.
#[async_trait]
pub trait PlanStateNotifier: Send + Sync {
    /// Called after a step status transition succeeds.
    ///
    /// `progress_summary` is the output of
    /// [`PlanState::progress_summary`](crate::plan_state::PlanState::progress_summary).
    async fn on_progress_changed(&self, progress_summary: &str);

    /// Called after a plan transitions to Completed state.
    ///
    /// Default implementation is a no-op for backward compatibility.
    async fn on_plan_completed(&self) {}
}

/// Default no-op implementation — useful for tests and contexts where
/// progress notification is not needed.
pub struct NoopNotifier;

#[async_trait]
impl PlanStateNotifier for NoopNotifier {
    async fn on_progress_changed(&self, _progress_summary: &str) {}
    async fn on_plan_completed(&self) {}
}
