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

use crate::bash::CommandSandbox;
use crate::permission_check::{
    check_command_permission, check_tool_permission, CommandPermissionResult, PermDeps,
};
use crate::security::{BashSecurityAnalyzer, TrustLevel};
use crate::{PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;

use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Mutex as TokioMutex;

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
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    bg_manager: Arc<dyn closeclaw_tasks::TaskManager>,
    session_manager: Arc<SessionManager>,
    config_manager: Arc<ConfigManager>,
    approval_flow: Arc<TokioMutex<ApprovalFlow>>,
}

impl BashTool {
    /// Creates a new `BashTool` backed by the given permission engine,
    /// background task manager, config manager, and approval flow.
    pub fn new(
        permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
        bg_manager: Arc<dyn closeclaw_tasks::TaskManager>,
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
        approval_flow: Arc<TokioMutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            permission_engine,
            bg_manager,
            session_manager,
            config_manager,
            approval_flow,
        }
    }

    /// Bundle permission dependencies into a [`PermDeps`] tuple.
    fn perm_deps(&self) -> PermDeps {
        (
            Arc::clone(&self.permission_engine),
            Arc::clone(&self.session_manager),
            Arc::clone(&self.config_manager),
            Arc::clone(&self.approval_flow),
        )
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
         execution. Commands exceeding 15s are auto-backgrounded. \
         Background tasks notify automatically on completion — do not poll. \
         Use run_in_background for commands expected to exceed 10 seconds."
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
                    "description": "Bypass sandbox restrictions (landlock + seccomp) for this command. Sandbox infrastructure is ready but enforcement strategy is implemented in a follow-up PR."
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
        execute_bash_call(&self.perm_deps(), &self.bg_manager, args, ctx).await
    }
}

// --- Helper functions ---

/// Truncate a command string for use as `args_summary` in pending
/// operation tracking. Caps at 200 characters to keep checkpoint
/// data compact.
fn truncate_summary(command: &str) -> String {
    const MAX_SUMMARY_LEN: usize = 200;
    if command.len() <= MAX_SUMMARY_LEN {
        command.to_string()
    } else {
        let end = command.floor_char_boundary(MAX_SUMMARY_LEN);
        format!("{}…", &command[..end])
    }
}

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

/// Awaits a [`tokio::sync::Notify`] if `Some`, or never resolves if `None`.
///
/// This helper lets `tokio::select!` branch on an optional signal:
/// when the signal is `None`, the branch is effectively disabled.
async fn notify_or_pending(signal: Option<&Arc<tokio::sync::Notify>>) {
    match signal {
        Some(s) => s.notified().await,
        None => std::future::pending::<()>().await,
    }
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

/// Backgroundize a child process and return the corresponding ToolResult.
///
/// Reattaches stdout/stderr handles before handing off to `bg_manager`.
/// When `by_user` is true, marks the result as `backgroundedByUser`.
async fn backgroundize_child(
    mut child: tokio::process::Child,
    stdout_handle: Option<tokio::process::ChildStdout>,
    stderr_handle: Option<tokio::process::ChildStderr>,
    command: &str,
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
    by_user: bool,
) -> Result<ToolResult, String> {
    child.stdout = stdout_handle;
    child.stderr = stderr_handle;
    let task = bg_manager
        .backgroundize_task(child, command, true)
        .await
        .map_err(|e| format!("failed to backgroundize command: {}", e))?;
    if by_user {
        Ok(build_manual_background_result(&task))
    } else {
        Ok(build_auto_background_result(&task))
    }
}

/// Wait on the child, then return either a normal result or auto-backgroundize.
async fn wait_child(
    wait_result: Result<
        Result<std::process::ExitStatus, std::io::Error>,
        tokio::time::error::Elapsed,
    >,
    child: tokio::process::Child,
    stdout_handle: Option<tokio::process::ChildStdout>,
    stderr_handle: Option<tokio::process::ChildStderr>,
    command: &str,
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
) -> Result<ToolResult, String> {
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
        Err(_elapsed) => {
            backgroundize_child(
                child,
                stdout_handle,
                stderr_handle,
                command,
                bg_manager,
                false,
            )
            .await
        }
    }
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
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
    manual_bg_signal: Option<&Arc<tokio::sync::Notify>>,
) -> Result<ToolResult, String> {
    let (stdout_handle, stderr_handle) = {
        let mut guard = child_arc.lock().expect("child mutex poisoned");
        let child = guard.as_mut().expect("child present after spawn");
        (child.stdout.take(), child.stderr.take())
    };
    let mut child = child_arc
        .lock()
        .expect("child mutex poisoned")
        .take()
        .expect("child present after spawn");
    tokio::select! {
        biased;
        _ = notify_or_pending(manual_bg_signal) => {
            backgroundize_child(
                child, stdout_handle, stderr_handle, command, bg_manager, true,
            ).await
        }
        result = tokio::time::timeout(bg_timeout, child.wait()) => {
            wait_child(result, child, stdout_handle, stderr_handle, command, bg_manager).await
        }
    }
}

/// Analyze command security. Returns `Err` if the command is untrusted.
fn analyze_security(command: &str) -> Result<(), ToolCallError> {
    let sec_result = BashSecurityAnalyzer::new()
        .map_err(ToolCallError::ExecutionFailed)?
        .analyze(command);
    if let TrustLevel::Uncertain | TrustLevel::Malicious = sec_result.trust_level {
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
    Ok(())
}

/// Check Level 2 command permission, routing through approval or sandbox.
///
/// When `dangerouslyDisable_sandbox` is true, the sandbox is bypassed
/// entirely even for denied commands.  Otherwise, denied commands have
/// landlock + seccomp restrictions applied before execution.
///
/// Returns `(Ok(Some(ToolResult)), true)` when routed to the approval flow,
/// `(Ok(None), true)` when the sandbox was already applied,
/// `(Ok(None), false)` when permitted (caller proceeds with normal
/// execution), or `Err` on security analysis errors.
///
/// The second element (`sandbox_applied`) tells the caller whether
/// sandbox restrictions were applied so they must not be applied again.
async fn check_command_permission_and_route(
    deps: &PermDeps,
    ctx: &ToolContext,
    command: &str,
    cmd_name: &str,
    cmd_args: &[String],
    dangerously_disable_sandbox: bool,
) -> Result<(Option<ToolResult>, bool), ToolCallError> {
    match check_command_permission(deps, ctx, cmd_name, cmd_args).await {
        CommandPermissionResult::Permitted => Ok((None, false)),
        CommandPermissionResult::PendingApproval(result) => Ok((Some(result), false)),
        CommandPermissionResult::Denied(reason) => {
            // Design doc: commands without permission are routed to the
            // sandbox for restricted execution, not directly rejected.
            tracing::info!(
                command = %command,
                reason = %reason,
                "Command denied by permission engine; routing to sandbox"
            );
            if !dangerously_disable_sandbox {
                let cwd = resolve_cwd(&serde_json::json!({}), ctx);
                CommandSandbox::apply_sandbox_restrictions(&cwd)?;
                return Ok((None, true));
            }
            // dangerouslyDisableSandbox=true: sandbox fully bypassed
            Ok((None, false))
        }
    }
}

/// Execute the BashTool call: parse args, check two-level permissions, run command.
async fn execute_bash_call(
    deps: &PermDeps,
    bg: &Arc<dyn closeclaw_tasks::TaskManager>,
    args: Value,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolCallError> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolCallError::InvalidArgs("missing required parameter: command".into()))?;
    if command.is_empty() {
        return Err(ToolCallError::InvalidArgs(
            "command must not be empty".into(),
        ));
    }
    analyze_security(command)?;

    // Level 1: ToolCall — verify agent may invoke Bash tool
    if let Some(r) = check_tool_permission(deps, ctx, "bash", "call").await? {
        return Ok(r);
    }

    let cmd_parts: Vec<&str> = command.split_whitespace().collect();
    let cmd_name = cmd_parts.first().copied().unwrap_or("*").to_string();
    let cmd_args: Vec<String> = cmd_parts[1..].iter().map(|s| s.to_string()).collect();

    let dangerously_disable = args.get("dangerouslyDisableSandbox") == Some(&Value::Bool(true));

    // Level 2: CommandExec — verify specific command is permitted.
    // Denied commands are routed to the sandbox (not rejected outright).
    let (approval_result, sandbox_already_applied) = check_command_permission_and_route(
        deps,
        ctx,
        command,
        &cmd_name,
        &cmd_args,
        dangerously_disable,
    )
    .await?;
    if let Some(r) = approval_result {
        return Ok(r);
    }

    // Sandbox routing: apply landlock + seccomp if needed.
    // Scripts are always sandboxed; permitted non-script commands run outside.
    // Skip if sandbox was already applied in the Level 2 denied path above,
    // or if dangerouslyDisableSandbox is true (full bypass).
    let cwd = resolve_cwd(&args, ctx);
    if !sandbox_already_applied
        && !dangerously_disable
        && CommandSandbox::should_sandbox(command, true)
    {
        CommandSandbox::apply_sandbox_restrictions(&cwd)?;
    }

    execute_command(
        command,
        &cwd,
        parse_timeout(&args),
        args.get("run_in_background") == Some(&Value::Bool(true)),
        bg,
        ctx.session.as_ref(),
        ctx.call_id.as_deref(),
        ctx.manual_background_signal.as_ref(),
    )
    .await
    .map_err(ToolCallError::ExecutionFailed)
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
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
    session: Option<&Arc<dyn closeclaw_common::tool_session::ToolSession>>,
    call_id: Option<&str>,
    manual_bg_signal: Option<&Arc<tokio::sync::Notify>>,
) -> Result<ToolResult, String> {
    if run_in_background {
        // ── Pending-operation tracking ──────────────────────────────────
        // Register the tool call before spawning so that a crash
        // after spawn but before completion is recoverable.
        let mut registered_call_id = None;
        if let (Some(s), Some(cid)) = (session, call_id) {
            let summary = truncate_summary(command);
            s.register_tool_call(cid.to_string(), "bash".to_string(), summary)
                .await;
            registered_call_id = Some(cid.to_string());
            s.persist_pending_checkpoint().await;
        }

        // Per #762 design: `spawn_task()` is the "self-cold-start" path; do not
        // pre-spawn a Child and pass it through `backgroundize_task()` here.
        let task = bg_manager
            .spawn_task(command, Path::new(cwd), false)
            .await
            .map_err(|e| {
                // Deregister on spawn failure.
                if let (Some(s), Some(cid)) = (session, registered_call_id.as_deref()) {
                    // Fire-and-forget deregister; spawn failure is the primary error.
                    let s = Arc::clone(s);
                    let cid = cid.to_string();
                    tokio::spawn(async move {
                        s.deregister_tool_call(cid).await;
                        s.persist_pending_checkpoint().await;
                    });
                }
                format!("failed to spawn background task: {}", e)
            })?;

        // Register BackgroundKillHandle so cascade-stop can find the
        // task. No unregister — background tasks run independently
        // of the tool invocation, and the handle is naturally
        // reaped when the entry is removed from the manager.
        if let (Some(s), Some(cid)) = (session, call_id) {
            let handle: Arc<dyn closeclaw_common::tool_session::KillHandle> =
                Arc::new(BackgroundKillHandle {
                    bg_manager: Arc::clone(bg_manager),
                    task_id: task.id.clone(),
                });
            s.register_tool_handle(cid.to_string(), handle).await;
        }

        // Deregister the pending-operation entry: the background task
        // runs independently of the tool invocation.
        if let (Some(s), Some(cid)) = (session, registered_call_id.as_deref()) {
            s.deregister_tool_call(cid.to_string()).await;
            s.persist_pending_checkpoint().await;
        }

        return Ok(build_background_result(&task));
    }

    // ── Pending-operation tracking (foreground) ──────────────────────────
    // Register before spawning so a crash during execution is
    // detectable by the recovery service.
    let mut registered_call_id = None;
    if let (Some(s), Some(cid)) = (session, call_id) {
        let summary = truncate_summary(command);
        s.register_tool_call(cid.to_string(), "bash".to_string(), summary)
            .await;
        registered_call_id = Some(cid.to_string());
        s.persist_pending_checkpoint().await;
    }

    // Foreground path: spawn, wrap child in a shared slot, register
    // the kill handle, wait, then unregister.
    let child = spawn_sh_command(command, cwd)?;
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));

    if let (Some(s), Some(cid)) = (session, call_id) {
        let handle: Arc<dyn closeclaw_common::tool_session::KillHandle> =
            Arc::new(BashKillHandle {
                child: Arc::clone(&child_arc),
            });
        s.register_tool_handle(cid.to_string(), handle).await;
    }

    let bg_timeout = if auto_backgroundize_excluded(command) {
        Duration::from_millis(timeout_ms)
    } else {
        Duration::from_millis(AUTO_BG_TIMEOUT_MS)
    };

    let result =
        handle_foreground_result(child_arc, command, bg_timeout, bg_manager, manual_bg_signal)
            .await;

    // ── Deregister pending operation ─────────────────────────────────────
    if let (Some(s), Some(cid)) = (session, registered_call_id.as_deref()) {
        s.deregister_tool_call(cid.to_string()).await;
        s.persist_pending_checkpoint().await;
    }

    // The kill handle's `Arc` is dropped here; if the foreground wait
    // consumed the child, the slot was already `None` and the drop is a no-op.
    // No explicit unregister needed — handle lifecycle is tied to the Arc.

    result
}

/// Build a [`ToolResult`] for an explicitly backgrounded command.
fn build_background_result(task: &closeclaw_tasks::BackgroundTask) -> ToolResult {
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
fn build_auto_background_result(task: &closeclaw_tasks::BackgroundTask) -> ToolResult {
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

/// Build a [`ToolResult`] for a manually backgrounded command.
fn build_manual_background_result(task: &closeclaw_tasks::BackgroundTask) -> ToolResult {
    ToolResult {
        data: serde_json::json!({
            "backgroundTaskId": task.id,
            "outputPath": task.output_path.to_string_lossy(),
            "backgroundedByUser": true,
        }),
        new_messages: vec![],
        context_modifier: None,
    }
}

#[cfg(test)]
#[path = "bash_tests.rs"]
mod tests;
