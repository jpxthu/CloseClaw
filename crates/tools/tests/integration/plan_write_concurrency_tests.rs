//! Cross-crate concurrency regression for issue #3158: tools Write/Edit
//! `plans/` writes — routed through the [`PlanFileAccess`] port into the
//! session's plan sidecar lock domain — interleaved with the session's
//! `touch_access_timestamp` (read-modify-write under the same lock) must
//! never tear the plan file apart. The final content stays complete with
//! a single well-formed access marker.
//!
//! Mirrors the session-side regression
//! (`plan_file_lock_tests::test_touch_and_locked_write_interleave_without_tearing`)
//! but drives the writer side through the real tools with the real port.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use closeclaw_session::plan_file as session_plan_file;
use closeclaw_tools::builtin::{EditTool, WriteTool};
use closeclaw_tools::plan_file_access::{PlanFileAccess, PlanResolveError};
use closeclaw_tools::{Tool, ToolContext};

const ROUNDS: usize = 50;
const MARKER_PREFIX: &str = "<!-- accessed: ";
const MARKER_SUFFIX: &str = " -->";

/// Full plan content including the access marker; every Write round
/// writes it verbatim.
const PLAN_BODY: &str = "# Interleave\n\n<!-- accessed: 2026-01-01T00:00:00+00:00 -->\n\n## Context\n\nalpha\n\n## Tasks\n\n- [ ] a\n- [ ] b\n\n## Verification\n\ndone\n";

/// Test port forwarding straight to the real session plan-file
/// functions — the same calls the daemon adapter makes in production.
struct RealSessionPlanFileAccess;

impl PlanFileAccess for RealSessionPlanFileAccess {
    fn resolve_plan_by_name(
        &self,
        _workdir: &Path,
        name: &str,
    ) -> Result<PathBuf, PlanResolveError> {
        Err(PlanResolveError::NotFound {
            name: name.to_string(),
        })
    }

    fn touch_access_timestamp(&self, plan_path: &Path) -> Result<(), std::io::Error> {
        session_plan_file::touch_access_timestamp(plan_path)
    }

    fn write_plan_file(&self, plan_path: &Path, content: &str) -> Result<(), std::io::Error> {
        session_plan_file::write_plan_file_locked(plan_path, content)
    }
}

fn make_ctx(workdir: &Path) -> ToolContext {
    ToolContext {
        agent_id: "master".to_string(),
        workdir: Some(closeclaw_tools::build_workdir_context(
            workdir.to_str().expect("utf-8 workdir"),
        )),
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

/// Seed `plans/x.md` (with the access marker) inside the workdir and
/// return the normalized absolute plan path — the path both the tools
/// (via workdir normalization) and the touch thread lock on.
fn seed_plan(workdir: &Path) -> PathBuf {
    let plan = workdir.join("plans").join("x.md");
    session_plan_file::write_plan_file_locked(&plan, PLAN_BODY).expect("seed write must succeed");
    plan
}

fn spawn_touch_thread(plan: PathBuf) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for _ in 0..ROUNDS {
            session_plan_file::touch_access_timestamp(&plan)
                .expect("touch must succeed in every round");
        }
    })
}

fn tool_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime")
}

/// Strip the (single, terminated) access marker so two contents compare
/// equal modulo the timestamp touch keeps refreshing. Panics on a torn
/// or unterminated marker — exactly the corruption this suite guards.
fn strip_access_marker(content: &str) -> String {
    let start = content
        .find(MARKER_PREFIX)
        .unwrap_or_else(|| panic!("access marker prefix missing, got: {content}"));
    let rest = &content[start..];
    let end = rest
        .find(MARKER_SUFFIX)
        .unwrap_or_else(|| panic!("access marker unterminated, got: {content}"))
        + start
        + MARKER_SUFFIX.len();
    let mut stripped = String::with_capacity(content.len());
    stripped.push_str(&content[..start]);
    stripped.push_str(&content[end..]);
    stripped
}

/// Final-state assertions shared by the Write and Edit scenarios: the
/// file is readable UTF-8, carries exactly one terminated marker, its
/// body matches one of the `expected_variants`, and the atomic write
/// left no `.tmp.*` leftovers next to the plan.
fn assert_plan_intact(plan: &Path, expected_variants: &[String]) {
    let final_content = std::fs::read_to_string(plan).expect("final plan readable");
    assert_eq!(
        final_content.matches(MARKER_PREFIX).count(),
        1,
        "exactly one access marker expected, got: {final_content}"
    );
    let stripped = strip_access_marker(&final_content);
    assert!(
        expected_variants.iter().any(|v| v == &stripped),
        "plan body torn or lost, got: {final_content}"
    );
    let leftovers: Vec<String> = std::fs::read_dir(plan.parent().expect("plans dir"))
        .expect("plans dir readable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic write left temp leftovers: {leftovers:?}"
    );
}

/// Writer thread drives the Write tool through the real port; the touch
/// thread uses the session lock domain directly. Both serialize on the
/// same sidecar lock, so the final file must be the complete body with
/// an intact marker — never a half-written overwrite.
#[test]
#[serial_test::serial]
fn test_plan_file_write_touch_interleave_without_tearing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let ctx = make_ctx(dir.path());
    let workdir = PathBuf::from(ctx.workdir.as_ref().expect("workdir").path.clone());
    let plan = seed_plan(&workdir);

    let port = Arc::new(RealSessionPlanFileAccess);
    let tool = Arc::new(WriteTool::new().with_plan_file_access(port));
    let rt = Arc::new(tool_runtime());
    let writer = std::thread::spawn(move || {
        for _ in 0..ROUNDS {
            rt.block_on(tool.call(
                serde_json::json!({ "path": "plans/x.md", "content": PLAN_BODY }),
                &ctx,
            ))
            .expect("plans write via port must succeed in every round");
        }
    });
    let toucher = spawn_touch_thread(plan.clone());
    writer.join().expect("writer thread must not panic");
    toucher.join().expect("touch thread must not panic");

    assert_plan_intact(&plan, &[strip_access_marker(PLAN_BODY)]);
}

/// Same scenario through Edit: the editor thread alternates the one
/// body bullet between its two states (so every round matches exactly
/// once regardless of interleaving) and writes the replaced content
/// back through the port.
#[test]
#[serial_test::serial]
fn test_plan_file_edit_touch_interleave_without_tearing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let ctx = make_ctx(dir.path());
    let workdir = PathBuf::from(ctx.workdir.as_ref().expect("workdir").path.clone());
    let plan = seed_plan(&workdir);

    let port = Arc::new(RealSessionPlanFileAccess);
    let tool = Arc::new(EditTool::new().with_plan_file_access(port));
    let rt = Arc::new(tool_runtime());
    let editor = std::thread::spawn(move || {
        for round in 0..ROUNDS {
            let (old, new) = if round % 2 == 0 {
                ("- [ ] a", "- [x] a")
            } else {
                ("- [x] a", "- [ ] a")
            };
            rt.block_on(tool.call(
                serde_json::json!({
                    "path": "plans/x.md",
                    "edits": [{ "oldText": old, "newText": new }],
                }),
                &ctx,
            ))
            .expect("plans edit via port must succeed in every round");
        }
    });
    let toucher = spawn_touch_thread(plan.clone());
    editor.join().expect("editor thread must not panic");
    toucher.join().expect("touch thread must not panic");

    let original = strip_access_marker(PLAN_BODY);
    let toggled = original.replace("- [ ] a", "- [x] a");
    assert_plan_intact(&plan, &[original, toggled]);
}
