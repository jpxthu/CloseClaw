//! Plan file store port for the slash crate.
//!
//! [`PlanFileStore`] is the slash-owned abstraction over plan file
//! operations (create / resolve / touch / list / read). Slash handlers
//! keep reply formatting and orchestration, while filesystem access is
//! delegated to this port — `closeclaw-session` types stay out of slash
//! production code.
//!
//! The composition root (daemon) supplies the production implementation
//! wrapping `closeclaw_session::plan_file`; tests may supply a thin
//! wrapper around the same functions (see the `real_store` test helper).
//!
//! Mirror types ([`PlanNameFormat`], [`PlanResolveError`],
//! [`PlanSummary`]) intentionally match their session-side counterparts
//! field-for-field and (for errors) Display-for-Display, so reply texts
//! are byte-identical to before the port boundary existed.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Format for plan file identifiers (slash-owned mirror).
///
/// Variants: [`PlanNameFormat::Timestamp`] is `yyyy-MM-dd-HH-mm-ss-{slug}`
/// (default); [`PlanNameFormat::RandomWords`] is
/// `{adjective}-{noun}-{noun}` (e.g. `calm-wave-oven`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlanNameFormat {
    /// Timestamp-based identifier (default).
    #[default]
    Timestamp,
    /// Random-words identifier.
    RandomWords,
}

/// Errors that can occur when resolving a plan file by name.
///
/// Display strings must stay identical to the session-side originals —
/// they are surfaced verbatim in `/execute` replies
/// (`计划文件解析失败：{e}`).
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

/// Summary information for a single plan file (`/plans` listing).
#[derive(Debug, Clone)]
pub struct PlanSummary {
    /// File stem (filename without `.md` extension).
    pub stem: String,
    /// Plan title extracted from the first heading line.
    pub title: String,
    /// Number of tasks completed (`[x]`).
    pub completed: usize,
    /// Number of tasks failed (`[!]`).
    pub failed: usize,
    /// Number of tasks skipped (`[~]`).
    pub skipped: usize,
    /// Total number of tasks (all checkbox lines in Tasks section).
    pub total: usize,
}

/// Slash-owned port over plan file operations.
///
/// Implementations must be `Send + Sync` (handlers are shared behind
/// `Arc`) and stateless with respect to any single session: all paths
/// cross the port explicitly.
pub trait PlanFileStore: Send + Sync {
    /// Create a plan file in `{workdir}/plans/` with the given
    /// identifier format. Returns the created file path.
    fn create_plan_file_with_format(
        &self,
        workdir: &Path,
        title: &str,
        format: PlanNameFormat,
    ) -> Result<PathBuf, std::io::Error>;

    /// Resolve a plan file path by name within a workspace
    /// (exact → prefix → fuzzy three-tier strategy).
    fn resolve_plan_by_name(&self, workdir: &Path, name: &str)
        -> Result<PathBuf, PlanResolveError>;

    /// Update (or insert) the application-layer access timestamp
    /// marker of a plan file.
    fn touch_access_timestamp(&self, plan_path: &Path) -> Result<(), std::io::Error>;

    /// List all plan summaries in `{workdir}/plans/`, sorted by
    /// modification time (most recent first). Missing plans directory
    /// yields an empty vector.
    fn list_plan_summaries(&self, workdir: &Path) -> std::io::Result<Vec<PlanSummary>>;

    /// Read the full content of a plan file at the given path.
    fn read_plan_content(&self, path: &Path) -> std::io::Result<String>;
}

/// Test-support thin port implementation wrapping the real
/// `closeclaw-session` plan-file functions (session is a
/// dev-dependency of this crate).
///
/// Only compiled for the slash crate's own tests; the production
/// implementation lives in the daemon composition root.
#[cfg(test)]
pub(crate) mod real_store {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use closeclaw_session::plan_file as session_plan_file;

    use super::{PlanFileStore, PlanNameFormat, PlanResolveError, PlanSummary};

    fn format_to_session(format: PlanNameFormat) -> session_plan_file::PlanIdentifierFormat {
        match format {
            PlanNameFormat::Timestamp => session_plan_file::PlanIdentifierFormat::Timestamp,
            PlanNameFormat::RandomWords => session_plan_file::PlanIdentifierFormat::RandomWords,
        }
    }

    fn error_to_slash(e: session_plan_file::PlanResolveError) -> PlanResolveError {
        match e {
            session_plan_file::PlanResolveError::NotFound { name } => {
                PlanResolveError::NotFound { name }
            }
            session_plan_file::PlanResolveError::Ambiguous { name, candidates } => {
                PlanResolveError::Ambiguous { name, candidates }
            }
        }
    }

    fn summary_to_slash(s: session_plan_file::PlanSummary) -> PlanSummary {
        PlanSummary {
            stem: s.stem,
            title: s.title,
            completed: s.completed,
            failed: s.failed,
            skipped: s.skipped,
            total: s.total,
        }
    }

    /// Thin wrapper around the real session plan-file functions.
    pub(crate) struct RealSessionPlanFileStore;

    impl PlanFileStore for RealSessionPlanFileStore {
        fn create_plan_file_with_format(
            &self,
            workdir: &Path,
            title: &str,
            format: PlanNameFormat,
        ) -> Result<PathBuf, std::io::Error> {
            session_plan_file::create_plan_file_with_format(
                workdir,
                title,
                format_to_session(format),
            )
        }

        fn resolve_plan_by_name(
            &self,
            workdir: &Path,
            name: &str,
        ) -> Result<PathBuf, PlanResolveError> {
            session_plan_file::resolve_plan_by_name(workdir, name).map_err(error_to_slash)
        }

        fn touch_access_timestamp(&self, plan_path: &Path) -> Result<(), std::io::Error> {
            session_plan_file::touch_access_timestamp(plan_path)
        }

        fn list_plan_summaries(&self, workdir: &Path) -> std::io::Result<Vec<PlanSummary>> {
            session_plan_file::list_plan_summaries(workdir)
                .map(|v| v.into_iter().map(summary_to_slash).collect())
        }

        fn read_plan_content(&self, path: &Path) -> std::io::Result<String> {
            session_plan_file::read_plan_content(path)
        }
    }

    /// Build a test store as `Arc<dyn PlanFileStore>`.
    pub(crate) fn test_store() -> Arc<dyn PlanFileStore> {
        Arc::new(RealSessionPlanFileStore)
    }
}
