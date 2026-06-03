//! Built-in BashTool — provides shell command execution capability for agents.
//! Implements timeout control, output truncation with head-preservation,
//! output persistence, and command classification.
//!
//! Step 1.4 of issue #858 added a kill-handle integration path: foreground
//! processes register a [`BashKillHandle`] on the owning
//! `ConversationSession`, background tasks register a
//! [`BackgroundKillHandle`]. The actual `KillHandle` adapter types and
//! the output-processing helpers (`process_output`, `build_result`,
//! etc.) live in the sibling module [`super::bash_kill`] to keep this
//! file under the CONTRIBUTING.md 500-line hard cap.

use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::PermissionResponse;
use crate::tasks::BackgroundTaskManager;
use crate::tools::security::{BashSecurityAnalyzer, TrustLevel};
use crate::tools::{
    PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags, ToolResult,
};

use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::bash_kill::{
    build_result, process_output, read_pipe, BackgroundKillHandle, BashKillHandle,
};

/// Auto-backgroundize timeout (15 seconds).
const AUTO_BG_TIMEOUT_MS: u64 = 15_000;

/// Shell command execution tool.
///
/// Receives a command string plus optional parameters (timeout, cwd,
/// description, run_in_background, dangerouslyDisableSandbox), validates
/// permissions via [`PermissionEngine`], then executes the command as
/// an async subprocess with timeout control.
pub struct BashTool {
    permission_engine: Arc<PermissionEngine>,
    bg_manager: Arc<BackgroundTaskManager>,
}

impl BashTool {
    /// Creates a new `BashTool` backed by the given permission engine
    /// and background task manager.
    pub fn new(
        permission_engine: Arc<PermissionEngine>,
        bg_manager: Arc<BackgroundTaskManager>,
    ) -> Self {
        Self {
            permission_engine,
            bg_manager,
        }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn group(&self) -> &str {
        "bash"
    }

    fn summary(&self) -> String {
        "Execute shell commands with timeout and output control".to_string()
    }

    fn detail(&self) -> String {
        "Execute a shell command via subprocess. Supports timeout control \
         (default 120s, max 600s), output truncation with head-preservation \
         (threshold 30,000 chars), and output persistence to disk when \
         output exceeds threshold. Supports run_in_background for async \
         execution. Commands exceeding 15s are auto-backgrounded."
            .to_string()
    }

    #[rustfmt::skip]
    fn generate_prompt(&self, context: &PromptGenerationContext) -> String {
        let Some(wd) = &context.workdir else { return self.detail() };
        let mut s = format!(" Working directory: {}.", wd.path);
        if wd.has_git {
            if let Some(b) = &wd.branch { s.push_str(&format!(" Branch: {}.", b)); }
            if wd.recent_changes > 0 {
                s.push_str(&format!(" {} uncommitted change(s).", wd.recent_changes));
            }
        }
        format!("{}{}", self.detail(), s)
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in milliseconds (default 120000, max 600000)"
                },
                "description": {
                    "type": "string",
                    "description": "Brief description of what this command does"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Run command in background, returns task ID immediately"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory (defaults to session workdir)"
                },
                "dangerouslyDisableSandbox": {
                    "type": "boolean",
                    "description": "Bypass sandbox restrictions (no-op, sandbox not implemented)"
                }
            },
            "required": ["command"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_destructive: true,
            is_expensive: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        execute_bash_call(&self.permission_engine, &self.bg_manager, args, ctx).await
    }
}

// --- Helper functions ---

/// Parse and clamp the `timeout` parameter. Default 120 000 ms, max 600 000 ms.
fn parse_timeout(args: &Value) -> u64 {
    let raw = args
        .get("timeout")
        .and_then(Value::as_f64)
        .unwrap_or(120_000.0);
    let ms = raw.max(0.0) as u64;
    ms.min(600_000)
}

/// Resolve the working directory for the subprocess.
/// Priority: explicit `cwd` arg > `ctx.workdir` > `std::env::current_dir()`.
fn resolve_cwd(args: &Value, ctx: &ToolContext) -> String {
    if let Some(cwd) = args.get("cwd").and_then(Value::as_str) {
        if !cwd.is_empty() {
            return cwd.to_string();
        }
    }
    if let Some(ref wd) = ctx.workdir {
        return wd.path.clone();
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/".to_string())
}

/// Returns true if the command should NOT be auto-backgrounded.
/// Sleep, true, false and variants are excluded from auto-backgrounding.
fn auto_backgroundize_excluded(command: &str) -> bool {
    let trimmed = command.trim();
    // Strip arguments (e.g., "sleep 5" → "sleep")
    let first_token = trimmed.split_whitespace().next().unwrap_or("");
    let base = std::path::Path::new(first_token)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(first_token);
    matches!(base, "sleep" | "true" | "false")
}

// --- Sub-execution helpers ---

/// Spawn a shell command as a child process.
fn spawn_sh_command(command: &str, cwd: &str) -> Result<tokio::process::Child, String> {
    use tokio::process::Command;
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn command: {}", e))
}

/// Wait on a foreground child process, with timeout.
///
/// The child is shared with the [`BashKillHandle`] via
/// `Arc<Mutex<Option<Child>>>`. Stdout/stderr are extracted first
/// (they need to be consumed independently of the wait); the child is
/// then taken out of the `Mutex` for the actual `child.wait()` call
/// — holding a `std::sync::Mutex` across an `.await` would either
/// deadlock a current-thread runtime or starve a multi-threaded
/// runtime's worker. While the child is "out", the `BashKillHandle`
/// is a no-op; the wait is expected to complete (foreground
/// commands are short) or be auto-backgroundized.
///
/// On timeout, hands the child back to the background task manager
/// (with stdout/stderr reattached).
async fn handle_foreground_result(
    child_arc: Arc<Mutex<Option<tokio::process::Child>>>,
    command: &str,
    bg_timeout: Duration,
    bg_manager: &Arc<BackgroundTaskManager>,
) -> Result<ToolResult, String> {
    // 1. Extract stdout/stderr (briefly lock). Reattach to the
    //    (still-`Some`) child in the slot for the auto-background
    //    path below.
    let (stdout_handle, stderr_handle) = {
        let mut guard = child_arc.lock().expect("child mutex poisoned");
        let child = guard.as_mut().expect("child present after spawn");
        (child.stdout.take(), child.stderr.take())
    };

    // 2. Take the child OUT of the `Mutex` for the `wait()` call.
    //    Lock is released immediately.
    let mut child = child_arc
        .lock()
        .expect("child mutex poisoned")
        .take()
        .expect("child present after spawn");

    let wait_result = tokio::time::timeout(bg_timeout, child.wait()).await;
    match wait_result {
        Ok(Ok(status)) => {
            let exit_code = status.code().unwrap_or(-1);
            let stdout_raw = read_pipe(stdout_handle).await;
            let stderr_raw = read_pipe(stderr_handle).await;
            let stdout_p = process_output(&stdout_raw);
            let stderr_p = process_output(&stderr_raw);
            Ok(build_result(command, stdout_p, stderr_p, exit_code, false))
        }
        Ok(Err(e)) => Err(format!("failed to wait on command: {}", e)),
        // Timeout: auto-backgroundize the running child
        Err(_elapsed) => {
            child.stdout = stdout_handle;
            child.stderr = stderr_handle;
            let task = bg_manager
                .backgroundize(child, command)
                .await
                .map_err(|e| format!("failed to backgroundize command: {}", e))?;
            Ok(build_auto_background_result(&task))
        }
    }
}

/// Execute the BashTool call: parse args, check permissions, run command.
async fn execute_bash_call(
    perm: &PermissionEngine,
    bg: &Arc<BackgroundTaskManager>,
    args: Value,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolCallError> {
    // --- 1. Parse parameters ---
    let err_msg = "missing required parameter: command";
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolCallError::InvalidArgs(err_msg.into()))?;
    if command.is_empty() {
        return Err(ToolCallError::InvalidArgs(
            "command must not be empty".into(),
        ));
    }

    let timeout_ms = parse_timeout(&args);
    let cwd = resolve_cwd(&args, ctx);
    let run_in_background = args.get("run_in_background") == Some(&Value::Bool(true));

    // `description` and `dangerouslyDisableSandbox` are ignored.
    let _ = args.get("description");
    let _ = args.get("dangerouslyDisableSandbox");

    // --- 2. Security analysis ---
    let mut analyzer = BashSecurityAnalyzer::new().map_err(ToolCallError::ExecutionFailed)?;
    let sec_result = analyzer.analyze(command);
    match sec_result.trust_level {
        TrustLevel::Trusted => {}
        TrustLevel::Uncertain | TrustLevel::Malicious => {
            return Err(ToolCallError::ExecutionFailed(format!(
                "Command {} (reason: {})",
                if sec_result.trust_level == TrustLevel::Malicious {
                    "blocked"
                } else {
                    "requires approval"
                },
                sec_result.reason.unwrap_or_default()
            )));
        }
    }

    // --- 3. Permission check ---
    if let PermissionResponse::Denied { reason, .. } = perm.check(&ctx.agent_id, "exec") {
        return Err(ToolCallError::PermissionDenied(reason));
    }

    // --- 4. Execute subprocess ---
    let result = execute_command(
        command,
        &cwd,
        timeout_ms,
        run_in_background,
        bg,
        ctx.session.as_ref(),
        ctx.call_id.as_deref(),
    )
    .await;
    match result {
        Ok(r) => Ok(r),
        Err(e) => Err(ToolCallError::ExecutionFailed(e)),
    }
}

/// Execute a shell command via `sh -c` with timeout.
///
/// When `run_in_background` is true, immediately spawns a background
/// task. Otherwise executes in foreground with a 15-second
/// auto-backgroundize budget.
///
/// `session` and `call_id` (when both `Some`) drive the kill-handle
/// integration from Step 1.4 of issue #858: the foreground path
/// registers a [`BashKillHandle`] for the duration of the wait, the
/// background path registers a [`BackgroundKillHandle`] for the
/// lifetime of the task. Both are `None`-safe — tool invocations
/// outside a tracked session (CLI, tests, prompt generation) skip
/// registration entirely.
#[allow(clippy::too_many_arguments)]
async fn execute_command(
    command: &str,
    cwd: &str,
    timeout_ms: u64,
    run_in_background: bool,
    bg_manager: &Arc<BackgroundTaskManager>,
    session: Option<&Arc<tokio::sync::RwLock<crate::llm::session::ConversationSession>>>,
    call_id: Option<&str>,
) -> Result<ToolResult, String> {
    if run_in_background {
        // Per #762 design: `spawn()` is the "self-cold-start" path; do not
        // pre-spawn a Child and pass it through `backgroundize()` here.
        let task = bg_manager
            .spawn(command, Path::new(cwd))
            .await
            .map_err(|e| format!("failed to spawn background task: {}", e))?;

        // Register BackgroundKillHandle so cascade-stop can find the
        // task. No unregister — background tasks run independently
        // of the tool invocation, and the handle is naturally
        // reaped when the entry is removed from the manager.
        if let (Some(s), Some(cid)) = (session, call_id) {
            let handle: Arc<dyn crate::llm::session::KillHandle> = Arc::new(BackgroundKillHandle {
                bg_manager: Arc::clone(bg_manager),
                task_id: task.id.clone(),
            });
            s.read().await.register_tool_handle(cid.to_string(), handle);
        }

        return Ok(build_background_result(&task));
    }

    // Foreground path: spawn, wrap child in a shared slot, register
    // the kill handle, wait, then unregister.
    let child = spawn_sh_command(command, cwd)?;
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));

    if let (Some(s), Some(cid)) = (session, call_id) {
        let handle: Arc<dyn crate::llm::session::KillHandle> = Arc::new(BashKillHandle {
            child: Arc::clone(&child_arc),
        });
        s.read().await.register_tool_handle(cid.to_string(), handle);
    }

    let bg_timeout = if auto_backgroundize_excluded(command) {
        Duration::from_millis(timeout_ms)
    } else {
        Duration::from_millis(AUTO_BG_TIMEOUT_MS)
    };

    let result = handle_foreground_result(child_arc, command, bg_timeout, bg_manager).await;

    // Unregister on both success and failure. The handle's `Arc` is
    // dropped here; if the foreground wait consumed the child, the
    // slot was already `None` and the drop is a no-op.
    if let (Some(s), Some(cid)) = (session, call_id) {
        s.read().await.unregister_tool_handle(cid);
    }

    result
}

/// Build a [`ToolResult`] for an explicitly backgrounded command.
fn build_background_result(task: &crate::tasks::BackgroundTask) -> ToolResult {
    ToolResult {
        data: serde_json::json!({
            "backgroundTaskId": task.id,
            "outputPath": task.output_path.to_string_lossy(),
        }),
        new_messages: vec![],
        context_modifier: None,
    }
}

/// Build a [`ToolResult`] for an auto-backgrounded command (15s timeout).
fn build_auto_background_result(task: &crate::tasks::BackgroundTask) -> ToolResult {
    ToolResult {
        data: serde_json::json!({
            "backgroundTaskId": task.id,
            "outputPath": task.output_path.to_string_lossy(),
            "assistantAutoBackgrounded": true,
        }),
        new_messages: vec![],
        context_modifier: None,
    }
}

#[cfg(test)]
#[path = "bash_tests.rs"]
mod tests;
