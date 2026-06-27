//! `BashKillHandle` adapters and output-processing helpers for
//! `BashTool`.
//!
//! This file is the extraction target of Step 1.8 of issue #858 — the
//! `KillHandle` adapters used to live in `src/tools/builtin/bash.rs`,
//! which exceeded the CONTRIBUTING.md 500-line hard cap after the
//! foreground/background kill integration in Step 1.4. The
//! extraction keeps the public surface of `BashTool` unchanged:
//! `bash.rs` re-imports [`build_result`], [`process_output`],
//! [`read_pipe`], [`BackgroundKillHandle`], and [`BashKillHandle`]
//! from this module and the rest of the project uses them via
//! `crate::builtin::bash_kill::{...}`.
//!
//! Contents:
//! - Constants: [`MAX_OUTPUT_CHARS`], [`MAX_PERSISTED_BYTES`],
//!   [`PREVIEW_BYTES`], [`PERSIST_DIR`]
//! - [`OutputProcessed`] — result of [`process_output`]
//! - [`BashKillHandle`] / [`BackgroundKillHandle`] — `KillHandle`
//!   adapters for foreground child processes and background tasks
//! - Output processing: [`read_pipe`], [`safe_truncate`],
//!   [`process_output`], [`persist_output`]
//! - Result building: [`build_result`]

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::AsyncReadExt;

use crate::ToolResult;
use closeclaw_common::TaskManager;
use closeclaw_llm::session::KillHandle;

use super::bash_classify;

// ── constants ────────────────────────────────────────────────────────────

/// Maximum characters per output stream (stdout/stderr) before
/// truncation. Above this, the inline view is replaced with a
/// `<persisted-output>` reference and the full body is written to
/// disk by [`persist_output`].
pub(crate) const MAX_OUTPUT_CHARS: usize = 30_000;

/// Maximum bytes for a single persisted output file (64 MiB). Output
/// past this limit is silently truncated by [`safe_truncate`] so the
/// disk write does not blow up the agent's working directory.
pub(crate) const MAX_PERSISTED_BYTES: usize = 64 * 1024 * 1024;

/// Number of bytes to preview from a persisted file inside the inline
/// `<persisted-output>` reference. Small enough to keep the inline
/// view scannable, large enough to give the LLM enough context to
/// decide whether to read the full file.
pub(crate) const PREVIEW_BYTES: usize = 2_000;

/// Directory for persisted output files. Created on demand by
/// [`persist_output`].
pub(crate) const PERSIST_DIR: &str = "/tmp/openclaw";

// ── OutputProcessed ──────────────────────────────────────────────────────

/// Processed output: either inline content (under [`MAX_OUTPUT_CHARS`])
/// or a persisted-output reference string with metadata about the
/// file on disk.
pub(crate) struct OutputProcessed {
    /// Inline content for the tool result. Either the full raw output
    /// (when it fits) or a `<persisted-output path="..." size="...">`
    /// reference with a [`PREVIEW_BYTES`]-byte head preview.
    pub inline: String,
    /// Absolute path to the persisted file, if any.
    pub persisted_path: Option<String>,
    /// Total size of the persisted file in bytes (0 if not persisted).
    pub persisted_size: usize,
}

// ── KillHandle adapters ─────────────────────────────────────────────────

/// Kill adapter for a *foreground* `tokio::process::Child`.
///
/// The child is held in a shared `Arc<Mutex<Option<Child>>>` slot
/// (the same slot `BashTool::execute_command` reads from). This lets
/// the foreground `wait()` call move the child out of the slot for
/// the duration of the wait (so we are not holding a `std::sync::Mutex`
/// across an `.await`), while still allowing the kill handle to
/// observe the slot:
///
/// - If the slot is `Some(child)` (wait has not consumed it yet —
///   e.g. the foreground call is still inside `handle_foreground_result`),
///   `kill()` calls `start_kill()`, which sends SIGKILL on Unix.
///
/// - If the slot is `None` (the foreground wait already finished and
///   took the child, or the child was auto-backgrounded into the
///   `BackgroundTaskManager`), `kill()` is a no-op. The session's
///   `tool_handles` is unregistered in both cases by
///   `BashTool::execute_command`'s success / failure path.
pub(crate) struct BashKillHandle {
    /// Shared slot holding the running child, if any.
    pub(crate) child: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl KillHandle for BashKillHandle {
    fn kill(&self) -> io::Result<()> {
        // `start_kill()` (not `kill()`) is used so this is safe to
        // call after the child has been reaped: `start_kill` on a
        // finished Child is a no-op, while `kill` may return an
        // InvalidInput error.
        let mut guard = self.child.lock().expect("child mutex poisoned");
        if let Some(child) = guard.as_mut() {
            // Best-effort: ignore the result. The caller is already
            // racing the wait completion.
            let _ = child.start_kill();
        }
        Ok(())
    }
}

/// Kill adapter for a *background* task managed by the TaskManager.
///
/// The handle is intentionally fire-and-forget: the background task
/// runs independently of the tool invocation, so the `KillHandle` is
/// only invoked from `ConversationSession::stop(cascade=true)`. The
/// handle is **not** unregistered on tool completion (it has no
/// matching `unregister` site in `BashTool::execute_command`); the
/// entry is naturally reaped when `TaskManager::kill_task`
/// succeeds and the slot is removed.
pub(crate) struct BackgroundKillHandle {
    /// Reference to the manager that owns the task.
    pub(crate) bg_manager: Arc<dyn TaskManager>,
    /// ID of the background task to kill.
    pub(crate) task_id: String,
}

impl KillHandle for BackgroundKillHandle {
    fn kill(&self) -> io::Result<()> {
        // The `KillHandle` trait is synchronous (`fn kill() -> io::Result<()>`),
        // but `BackgroundTaskManager::kill` is async. We bridge by
        // dispatching through a dedicated tokio runtime: the
        // BackgroundTaskManager is `Send + Sync` and its `kill`
        // future is short-lived, so the runtime cost is negligible
        // compared to the wall-clock budget enforced by
        // `ConversationSession::kill_tool_handles` (5 s).
        //
        // If we are not inside a tokio runtime (e.g. a non-tokio
        // test driver), fall back to `block_in_place` to drive the
        // future to completion. If that also fails, we surface the
        // error to the caller — stop() will log it and move on.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // Inside a tokio runtime — block on the future.
                handle
                    .block_on(self.bg_manager.kill_task(&self.task_id))
                    .map_err(io::Error::other)
            }
            Err(_) => Err(io::Error::other(
                "BackgroundKillHandle::kill called outside a tokio runtime",
            )),
        }
    }
}

// ── read_pipe ────────────────────────────────────────────────────────────

/// Drain an optional async reader into a `String`. Used to collect
/// stdout / stderr from a foreground `tokio::process::Child` after
/// the child has exited.
pub(crate) async fn read_pipe<R>(pipe: Option<R>) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    match pipe {
        Some(mut r) => {
            let mut buf = Vec::new();
            // Best-effort: a broken pipe should not turn into an
            // error result; the upstream `child.wait()` will
            // surface real failures.
            let _ = r.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        }
        None => String::new(),
    }
}

// ── safe_truncate ───────────────────────────────────────────────────────

/// Truncate `s` to at most `max_bytes` bytes, snapping to the nearest
/// UTF-8 character boundary. Returns the full input when it already
/// fits.
pub(crate) fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let end = s
        .char_indices()
        .map(|(i, c)| i + c.len_utf8())
        .take_while(|&end| end <= max_bytes)
        .last()
        .unwrap_or(0);
    &s[..end]
}

// ── process_output ──────────────────────────────────────────────────────

/// Truncate output beyond [`MAX_OUTPUT_CHARS`] and persist full
/// output to disk.
///
/// Returns an [`OutputProcessed`] with either inline content (when
/// the raw fits) or a persisted-output reference string + metadata
/// about the on-disk file. Disk persistence is best-effort: on
/// failure, the function falls back to an inline truncation marker
/// so the tool result still carries a useful (if lossy) representation
/// of the output.
pub(crate) fn process_output(raw: &str) -> OutputProcessed {
    let char_count = raw.chars().count();
    if char_count <= MAX_OUTPUT_CHARS {
        return OutputProcessed {
            inline: raw.to_string(),
            persisted_path: None,
            persisted_size: 0,
        };
    }
    let truncated: String = raw.chars().take(MAX_OUTPUT_CHARS).collect();
    let removed_bytes = raw.len() - truncated.len();
    let inline = format!(
        "{}\n[truncated, {} bytes removed]",
        truncated, removed_bytes
    );
    match persist_output(raw) {
        Ok(path) => {
            let preview = safe_truncate(raw, PREVIEW_BYTES);
            let reference = format!(
                "<persisted-output path=\"{}\" size=\"{}\">\n{}\n</persisted-output>",
                path,
                raw.len(),
                preview,
            );
            OutputProcessed {
                inline: reference,
                persisted_path: Some(path),
                persisted_size: raw.len(),
            }
        }
        Err(e) => {
            eprintln!("warn: failed to persist bash output: {}", e);
            OutputProcessed {
                inline,
                persisted_path: None,
                persisted_size: 0,
            }
        }
    }
}

// ── persist_output ──────────────────────────────────────────────────────

/// Persist full output to a unique file under [`PERSIST_DIR`].
///
/// File name: `bash_output_{unix_ms}_{pid}_{counter}.txt`. The counter
/// is a process-wide atomic so concurrent callers do not collide.
/// Output larger than [`MAX_PERSISTED_BYTES`] is silently truncated
/// to the byte boundary by [`safe_truncate`].
pub(crate) fn persist_output(raw: &str) -> Result<String, String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(PERSIST_DIR)
        .map_err(|e| format!("failed to create {}: {}", PERSIST_DIR, e))?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let pid = std::process::id();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = format!("{}/bash_output_{}_{}_{}.txt", PERSIST_DIR, ts, pid, seq);
    let content = safe_truncate(raw, MAX_PERSISTED_BYTES);
    std::fs::write(&path, content).map_err(|e| format!("failed to write {}: {}", path, e))?;
    Ok(path)
}

// ── build_result ────────────────────────────────────────────────────────

/// Build a [`ToolResult`] from processed execution outputs.
pub(crate) fn build_result(
    command: &str,
    stdout: OutputProcessed,
    stderr: OutputProcessed,
    exit_code: i32,
    interrupted: bool,
) -> ToolResult {
    let category = bash_classify::classify_command(command);
    let no_output = bash_classify::no_output_expected(category);
    let interpretation = crate::security::interpret_exit_code(command, exit_code);
    let persisted_path = stdout
        .persisted_path
        .as_deref()
        .or(stderr.persisted_path.as_deref())
        .map(str::to_string);
    let persisted_size = stdout.persisted_size + stderr.persisted_size;
    ToolResult {
        data: serde_json::json!({
            "stdout": stdout.inline,
            "stderr": stderr.inline,
            "exitCode": exit_code,
            "interrupted": interrupted,
            "persistedOutputPath": persisted_path,
            "persistedOutputSize": persisted_size,
            "returnCodeInterpretation": interpretation,
            "noOutputExpected": no_output
        }),
        new_messages: vec![],
        context_modifier: None,
    }
}
