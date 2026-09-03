//! Trait for plan execution confirmation handling.
//!
//! Gateway uses this trait to intercept `/confirm` and `/cancel` commands
//! without depending on the `closeclaw-tools` crate (where
//! [`PlanExecConfirmFlow`](closeclaw_tools::builtin::PlanExecConfirmFlow)
//! is defined).

/// Trait abstracting plan execution confirmation operations.
///
/// Implemented by `PlanExecConfirmFlow` in the tools crate; consumed by
/// Gateway via `Arc<dyn PlanConfirmationHandler>`.
#[async_trait::async_trait]
pub trait PlanConfirmationHandler: Send + Sync {
    /// Confirm a pending plan execution.
    ///
    /// Returns `true` if processed, `false` if unknown/already consumed.
    async fn confirm(&self, confirmation_id: &str) -> bool;

    /// Cancel a pending plan execution.
    ///
    /// Returns `true` if cancelled, `false` if unknown.
    async fn cancel(&self, confirmation_id: &str) -> bool;
}
