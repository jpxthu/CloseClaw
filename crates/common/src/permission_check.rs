//! Permission check traits and error types for cross-crate use.
//!
//! Holds the [`PermissionDenied`] error consumed by `closeclaw-execution`
//! plus the spawn-side [`PermissionChecker`] / [`SpawnPermissionError`]
//! pair. [`PermissionChecker`] is implemented in `closeclaw-gateway`
//! (session tests carry their own mock); this module only holds trait
//! signatures and the error type.

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

// ── Spawn permission checking ───────────────────────────────────────────

/// Error returned when a spawn permission check is denied.
#[derive(Debug, Clone, Error)]
pub enum SpawnPermissionError {
    #[error("spawn permission denied for agent '{agent_id}': {reason}")]
    Denied { agent_id: String, reason: String },
}

/// Trait for checking whether a child agent is permitted to spawn
/// under a given parent session.
///
/// Implementations live in the gateway crate (wrapping `PermissionEngine`);
/// the session crate consumes this trait through `closeclaw-common` to avoid
/// a circular dependency on `closeclaw-permission`.
#[async_trait::async_trait]
pub trait PermissionChecker: Send + Sync {
    /// Validate that `child_agent_id` is permitted to spawn as a child
    /// of `parent_session_id`.
    ///
    /// Returns `Ok(())` if the spawn is permitted, or
    /// `Err(SpawnPermissionError::Denied)` with a reason if not.
    async fn validate_spawn_permission(
        &self,
        child_agent_id: &str,
        parent_session_id: &str,
    ) -> Result<(), SpawnPermissionError>;
}
