//! Permission check trait for the execution engine.
//!
//! Defines the [`ExecutionPermissionCheck`] trait so that the execution
//! engine can enforce permission policies without depending on the
//! permission crate. Implementations live in `closeclaw-permission`
//! (or in test mocks); this module only holds the trait signature —
//! the [`PermissionDenied`] error type stays in `closeclaw-common`.

use closeclaw_common::PermissionDenied;

/// Trait for checking whether a step is permitted to execute.
///
/// Implementations live in the permission crate; the execution crate consumes
/// this trait directly to avoid a circular dependency.
#[async_trait::async_trait]
pub trait ExecutionPermissionCheck: Send + Sync {
    /// Check whether the step described by `step_description` is allowed to run.
    ///
    /// Returns `Ok(())` if the step is permitted, or
    /// `Err(PermissionDenied)` with a reason if not.
    async fn check_execution(&self, step_description: &str) -> Result<(), PermissionDenied>;
}
