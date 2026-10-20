//! Plans-path routing for Write/Edit payloads.
//!
//! Decides where a file write is committed: `plans/` targets route
//! through the injected [`PlanFileAccess`] port (session sidecar lock +
//! atomic write), everything else keeps the plain readback write.
//! Split out of `file_ops.rs` as a single-responsibility helper
//! (issue #3817).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::plan_file_access::PlanFileAccess;
use crate::{ToolCallError, ToolContext, ToolResult};

/// Check if a file path belongs to the plans/ directory.
///
/// Isomorphic to the permission engine's `is_plans_path`
/// (`crates/permission/src/engine/engine_mode_filter.rs`) so tool-side
/// and permission-side agree on what counts as a plan file. Only the
/// string form is checked — this runs before workdir normalization,
/// which is exactly how permission sees the raw argument too.
///
/// The match must happen on the raw path, but the subsequent port write
/// needs an absolute path: the session-side sidecar lock key is the
/// path verbatim + ".lock", so a relative path would derive its lock
/// file from the process CWD and split the lock domain from the
/// session's plan writers.
pub(crate) fn is_plan_file_path(path: &str) -> bool {
    path.starts_with("plans/") || path.contains("/plans/")
}

/// Shared routing judgment: plans-path detection followed by workdir
/// normalization, `None` for non-plans targets.
fn normalized_plan_path(path: &str, ctx: &ToolContext) -> Option<PathBuf> {
    is_plan_file_path(path)
        .then(|| crate::workdir_context::normalize_against_workdir(path, ctx.workdir.as_ref()))
}

/// Where a Write/Edit payload is committed.
///
/// Makes the two sinks explicit so no parameter is silently discarded:
/// [`WriteSink::Plan`] routes through the injected [`PlanFileAccess`]
/// port, [`WriteSink::Readback`] keeps the plain readback write.
pub(crate) enum WriteSink<'a> {
    /// `plans/` target — the path must already be workdir-normalized
    /// (absolute) so the session sidecar lock key matches the session's
    /// writers.
    Plan(&'a Path),
    /// Regular target — original readback behavior, unchanged.
    Readback(&'a Path),
}

impl<'a> WriteSink<'a> {
    /// Pick the sink from the normalized plans path: `Some` → port,
    /// `None` → readback.
    pub(crate) fn pick(plan_path: Option<&'a Path>, raw_path: &'a Path) -> Self {
        match plan_path {
            Some(plan) => WriteSink::Plan(plan),
            None => WriteSink::Readback(raw_path),
        }
    }
}

/// Commit `content` to `sink`.
///
/// `Plan` targets go through the injected [`PlanFileAccess`] port so the
/// write shares the session's plan sidecar lock domain and is atomic
/// (tempfile + rename) — the plain readback write is not safe against a
/// concurrent session touch. A missing port is a fail-fast wiring error:
/// silently falling back would lose that protection. `Readback` targets
/// keep the original readback write unchanged.
pub(crate) fn commit_to_sink(
    sink: WriteSink<'_>,
    content: &str,
    plan_file_access: Option<&Arc<dyn PlanFileAccess>>,
) -> Result<(), ToolCallError> {
    match sink {
        WriteSink::Readback(target) => super::readback::write_with_readback(target, content),
        WriteSink::Plan(plan_path) => {
            let port = plan_file_access.ok_or_else(|| {
                ToolCallError::ExecutionFailed(
                    "plan file writes require an injected PlanFileAccess port \
                     (build the tool via with_plan_file_access)"
                        .to_string(),
                )
            })?;
            port.write_plan_file(plan_path, content).map_err(|e| {
                ToolCallError::ExecutionFailed(format!("{}: {e}", plan_path.display()))
            })
        }
    }
}

/// Write content to a file, creating parent directories as needed.
///
/// `plans/` targets are routed through the injected
/// [`PlanFileAccess`] port via [`commit_to_sink`].
pub(crate) async fn write_file(
    path: &str,
    content: &str,
    ctx: &ToolContext,
    plan_file_access: Option<&Arc<dyn PlanFileAccess>>,
) -> Result<ToolResult, ToolCallError> {
    let plan_path = normalized_plan_path(path, ctx);
    if plan_path.is_none() {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ToolCallError::ExecutionFailed(format!("create_dir_all: {e}")))?;
        }
    }
    let sink = WriteSink::pick(plan_path.as_deref(), Path::new(path));
    commit_to_sink(sink, content, plan_file_access)?;
    Ok(ToolResult {
        data: serde_json::json!({ "content": content }),
        new_messages: vec![],
        context_modifier: None,
    })
}

/// Routing decision for one `Edit` payload.
///
/// Bundles the routing judgments `EditTool::call` performs inline —
/// plans-path detection, workdir normalization, and
/// plan_path/read_path derivation — so the read and the committed
/// write can never disagree on the target file. The sink is derived
/// from the same decision via [`EditRoute::sink`].
pub(crate) struct EditRoute {
    /// Workdir-normalized plans path (`None` for non-plans targets).
    plan_path: Option<PathBuf>,
    /// Path the edit reads the current content from: normalized for
    /// plans targets, raw argument otherwise.
    read_path: String,
}

impl EditRoute {
    /// Decide the route for the raw `path` argument.
    pub(crate) fn decide(path: &str, ctx: &ToolContext) -> Self {
        // plans/ targets read AND write through the workdir-normalized
        // path: when the process CWD differs from the session workdir,
        // reading the raw relative path while writing the normalized
        // one would target different files.
        let plan_path = normalized_plan_path(path, ctx);
        let read_path = plan_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        Self {
            plan_path,
            read_path,
        }
    }

    /// Path the edited file's current content is read from.
    pub(crate) fn read_path(&self) -> &str {
        &self.read_path
    }

    /// Sink committing the edited content, derived from the same
    /// routing decision; commit it via [`commit_to_sink`].
    pub(crate) fn sink<'a>(&'a self, raw_path: &'a str) -> WriteSink<'a> {
        WriteSink::pick(self.plan_path.as_deref(), Path::new(raw_path))
    }
}
