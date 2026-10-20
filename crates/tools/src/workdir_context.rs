//! WorkdirContext — working directory context for tools.
//!
//! Re-exports from `closeclaw_common::tool_trait` for backward compatibility
//! and hosts the crate-shared workdir path normalization.

use std::path::{Path, PathBuf};

pub use closeclaw_common::tool_trait::{
    build_git_status_for, build_workdir_context, WorkdirContext,
};

/// Normalize a possibly-relative path against the session workdir.
///
/// Absolute paths pass through untouched; relative paths resolve against
/// the workdir when one is set, and stay relative otherwise (the
/// process-CWD fallback, matching how `std::fs` treats relative inputs).
///
/// Crate-shared single definition (used by the Write/Edit plans/ branch
/// and the ModeExecutionTrigger touch). The result feeds the session
/// plan sidecar lock key — the path verbatim + ".lock" — so callers that
/// hand the result to a plan write must supply a workdir whenever
/// possible, or the lock domain splits from the session's writers.
pub(crate) fn normalize_against_workdir(path: &str, workdir: Option<&WorkdirContext>) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match workdir {
        Some(wd) => Path::new(&wd.path).join(p),
        None => p.to_path_buf(),
    }
}
