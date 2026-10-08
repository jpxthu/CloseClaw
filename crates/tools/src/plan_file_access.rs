//! Plan file access port for the tools crate.
//!
//! [`PlanFileAccess`] is the tools-owned abstraction over plan file
//! operations, narrowed to the exact call surface of
//! [`crate::builtin::mode_execution_trigger::ModeExecutionTriggerTool`]:
//! resolve a plan file by name within a workspace, and refresh its
//! application-layer access timestamp. Session-side plan state reads go
//! through the existing [`closeclaw_common::SessionLookup`] trait, so
//! `closeclaw-session` types stay out of tools production code.
//!
//! The composition root (daemon) supplies the production implementation
//! wrapping `closeclaw_session::plan_file`; tests may use the thin
//! wrapper in the `real_access` test helper (session is a
//! dev-dependency of this crate).
//!
//! The mirror type ([`PlanResolveError`]) intentionally matches its
//! session-side counterpart field-for-field and Display-for-Display,
//! so `ToolCallError` messages are byte-identical to before the port
//! boundary existed.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur when resolving a plan file by name.
///
/// Display strings must stay identical to the session-side originals —
/// they are surfaced verbatim in `ToolCallError::InvalidArgs` messages.
#[derive(Debug, Error)]
pub enum PlanResolveError {
    /// No plan file matched the given name.
    #[error("plan not found: {name}")]
    NotFound {
        /// The name that was searched for.
        name: String,
    },

    /// Multiple plan files matched the given name.
    #[error("ambiguous plan name '{name}': {candidates:?}")]
    Ambiguous {
        /// The name that was searched for.
        name: String,
        /// Stems of all matching plan files.
        candidates: Vec<String>,
    },
}

/// Tools-owned port over plan file operations.
///
/// Implementations must be `Send + Sync` (tools are shared behind
/// `Arc`).
pub trait PlanFileAccess: Send + Sync {
    /// Resolve a plan file path by name within a workspace
    /// (exact → prefix → fuzzy three-tier strategy).
    fn resolve_plan_by_name(&self, workdir: &Path, name: &str)
        -> Result<PathBuf, PlanResolveError>;

    /// Update (or insert) the application-layer access timestamp
    /// marker of a plan file so it is not archived prematurely.
    fn touch_access_timestamp(&self, plan_path: &Path) -> Result<(), std::io::Error>;
}

/// Test-support thin port implementation wrapping the real
/// `closeclaw_session::plan_file` functions (session is a
/// dev-dependency of this crate).
///
/// Only compiled for the tools crate's own tests; the production
/// implementation lives in the daemon composition root.
#[cfg(test)]
pub(crate) mod real_access {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::PlanFileAccess;

    /// Map a session-side resolve error to the tools-side mirror,
    /// preserving the payload (and therefore the `Display` output)
    /// verbatim.
    ///
    /// NOTE: this mapping must stay in sync variant-by-variant with the
    /// daemon's production mapping (`closeclaw_daemon::
    /// plan_file_store_adapter::error_to_tools`); new `PlanResolveError`
    /// variants added on either side require updating both.
    pub(crate) fn resolve_error_to_mirror(
        e: closeclaw_session::plan_file::PlanResolveError,
    ) -> super::PlanResolveError {
        match e {
            closeclaw_session::plan_file::PlanResolveError::NotFound { name } => {
                super::PlanResolveError::NotFound { name }
            }
            closeclaw_session::plan_file::PlanResolveError::Ambiguous { name, candidates } => {
                super::PlanResolveError::Ambiguous { name, candidates }
            }
        }
    }

    /// Thin wrapper around the real session plan-file functions.
    pub(crate) struct RealPlanFileAccess;

    impl PlanFileAccess for RealPlanFileAccess {
        fn resolve_plan_by_name(
            &self,
            workdir: &Path,
            name: &str,
        ) -> Result<PathBuf, super::PlanResolveError> {
            closeclaw_session::plan_file::resolve_plan_by_name(workdir, name)
                .map_err(resolve_error_to_mirror)
        }

        fn touch_access_timestamp(&self, plan_path: &Path) -> Result<(), std::io::Error> {
            closeclaw_session::plan_file::touch_access_timestamp(plan_path)
        }
    }

    /// Build the real plan-file access as `Arc<dyn PlanFileAccess>`.
    pub(crate) fn real_plan_file_access() -> Arc<dyn PlanFileAccess> {
        Arc::new(RealPlanFileAccess)
    }
}
