//! Permission check trait for execution engine integration.
//!
//! Defines the [`ExecutionPermissionCheck`] trait so that `closeclaw-execution`
//! can enforce permission policies without depending on the permission crate.
//! Implementations live in `closeclaw-permission`; this module only holds the
//! trait signature and the error type.

use std::fmt;

use thiserror::Error;

/// Error returned when an execution permission check is denied.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub struct PermissionDenied {
    /// Human-readable reason the permission was denied.
    pub reason: String,
}

impl fmt::Display for PermissionDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "permission denied: {}", self.reason)
    }
}

impl PermissionDenied {
    /// Create a new denial with the given reason.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Trait for checking whether a step is permitted to execute.
///
/// Implementations live in the permission crate; the execution crate consumes
/// this trait through `closeclaw-common` to avoid a circular dependency.
#[async_trait::async_trait]
pub trait ExecutionPermissionCheck: Send + Sync {
    /// Check whether the step described by `step_description` is allowed to run.
    ///
    /// Returns `Ok(())` if the step is permitted, or
    /// `Err(PermissionDenied)` with a reason if not.
    async fn check_execution(&self, step_description: &str) -> Result<(), PermissionDenied>;
}
