use crate::permission_check::PermDeps;
use crate::{ToolCallError, ToolContext, ToolResult};
use closeclaw_common::ToolExecState;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::trust::{analyze_security, prepare_and_sandbox, route_trust_level, TrustDecision};
use crate::builtin::bash_kill::{
    build_auto_background_result, build_background_result, build_manual_background_result,
    finalize_foreground_after_wait, register_foreground_session, truncate_summary,
    BackgroundKillHandle,
};

/// Outcome of foreground tool execution: normal completion or auto-backgroundized.
#[derive(Debug)]
pub(crate) enum ForegroundOutcome {
    /// Tool completed normally (success or non-zero exit).
    Completed(ToolResult),
    /// Tool execution error (e.g. spawn failure, wait failure).
    Failed(String),
    /// Tool was auto-backgroundized on timeout.
    /// Contains the ToolResult and the background task ID.
    AutoBackground(ToolResult, String),
}

/// Auto-backgroundize timeout (15 seconds).
pub(crate) const AUTO_BG_TIMEOUT_MS: u64 = 15_000;

/// Maximum agent-requested timeout (10 min). Prevents excessively long auto-backgroundize.
pub(crate) const AUTO_BG_TIMEOUT_CAP_MS: u64 = 600_000;

/// Parse and clamp agent timeout (None if absent, max 600s).
pub(crate) fn parse_timeout(args: &Value) -> Option<u64> {
    let raw = args.get("timeout").and_then(Value::as_f64)?;
    let ms = raw.max(0.0) as u64;
    Some(ms.min(600_000))
}

/// Resolve the working directory for the subprocess.
/// Priority: explicit `cwd` arg > `ctx.workdir` > `std::env::current_dir()`.
pub(crate) fn resolve_cwd(args: &Value, ctx: &ToolContext) -> String {
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
pub(crate) fn auto_backgroundize_excluded(command: &str) -> bool {
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
pub(crate) fn spawn_sh_command(command: &str, cwd: &str) -> Result<tokio::process::Child, String> {
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

/// Handles owned by a spawned child process: the child itself plus its
/// piped stdout/stderr streams.
///
/// The three always move together through the backgroundize chain
/// (`backgroundize_child` / `auto_backgroundize_foreground`) and the
/// foreground result-handling chain (`handle_timeout_expiry`), so they are
/// grouped to keep those signatures within the CONTRIBUTING 6-parameter cap.
pub(crate) struct ChildHandles {
    pub(crate) child: tokio::process::Child,
    pub(crate) stdout_handle: Option<tokio::process::ChildStdout>,
    pub(crate) stderr_handle: Option<tokio::process::ChildStderr>,
}

/// Backgroundize a child process and return the corresponding ToolResult.
///
/// Reattaches stdout/stderr handles before handing off to `bg_manager`.
/// When `by_user` is true, marks the result as `backgroundedByUser`.
pub(crate) async fn backgroundize_child(
    handles: ChildHandles,
    command: &str,
    bg_manager: &Arc<dyn closeclaw_common::TaskManager>,
    by_user: bool,
    session_id: &str,
) -> Result<(ToolResult, String), String> {
    let ChildHandles {
        mut child,
        stdout_handle,
        stderr_handle,
    } = handles;
    child.stdout = stdout_handle;
    child.stderr = stderr_handle;
    let task = bg_manager
        .backgroundize_task(child, command, true, session_id)
        .await
        .map_err(|e| format!("failed to backgroundize command: {}", e))?;
    let task_id = task.id.clone();
    if by_user {
        Ok((build_manual_background_result(&task), task_id))
    } else {
        Ok((build_auto_background_result(&task), task_id))
    }
}
/// Auto-backgroundize a foreground child and return a [`ForegroundOutcome`].
///
/// On failure, returns [`ForegroundOutcome::Failed`].
pub(crate) async fn auto_backgroundize_foreground(
    handles: ChildHandles,
    command: &str,
    bg_manager: &Arc<dyn closeclaw_common::TaskManager>,
    by_user: bool,
    session_id: &str,
) -> ForegroundOutcome {
    match backgroundize_child(handles, command, bg_manager, by_user, session_id).await {
        Ok((result, task_id)) => ForegroundOutcome::AutoBackground(result, task_id),
        Err(e) => ForegroundOutcome::Failed(e),
    }
}

/// Context for foreground result handling.
pub(crate) struct ForegroundContext<'a> {
    /// Shared execution context (command, cwd, bg_manager, session,
    /// call_id, session_id) flowing unchanged through the fg chain.
    pub(crate) exec: &'a BashExecCtx<'a>,
    pub(crate) manual_bg_signal: Option<&'a Arc<tokio::sync::Notify>>,
    /// When true, a timeout should force-kill the child (not auto-backgroundize).
    /// Set for whitelist commands on the total-execution-time fallback path.
    pub(crate) force_terminate: bool,
}

/// Handle a timeout expiry: force-terminate or auto-backgroundize.
pub(crate) async fn handle_timeout_expiry(
    handles: ChildHandles,
    command: &str,
    ctx: &ForegroundContext<'_>,
) -> ForegroundOutcome {
    if ctx.force_terminate {
        let ChildHandles {
            mut child,
            stdout_handle,
            stderr_handle,
        } = handles;
        let _ = child.start_kill();
        // Drain pipes so progress monitor can report output before the kill.
        let _ = crate::builtin::bash_kill::read_with_progress(
            stdout_handle,
            stderr_handle,
            ctx.exec.session,
            ctx.exec.call_id,
        )
        .await;
        ForegroundOutcome::Failed(format!(
            "Command '{}' exceeded total execution time limit \
             and was killed",
            command,
        ))
    } else {
        auto_backgroundize_foreground(
            handles,
            command,
            ctx.exec.bg_manager,
            false,
            ctx.exec.session_id,
        )
        .await
    }
}

/// Wait on a foreground child process, with timeout.
///
/// The child is shared with `BashKillHandle` via `Arc<Mutex<Option<Child>>>`.
/// Stdout/stderr are extracted first; the child is then taken out of the
/// `Mutex` for `child.wait()` — holding a std Mutex across await would deadlock.
/// On timeout, auto-backgroundizes or force-kills per `force_terminate` flag.
pub(crate) async fn handle_foreground_result(
    child_arc: Arc<Mutex<Option<tokio::process::Child>>>,
    command: &str,
    bg_timeout: Duration,
    ctx: &ForegroundContext<'_>,
) -> ForegroundOutcome {
    let (stdout_handle, stderr_handle) = {
        let mut guard = child_arc.lock().expect("child mutex poisoned");
        let child = guard.as_mut().expect("child present after spawn");
        (child.stdout.take(), child.stderr.take())
    };
    let child = child_arc
        .lock()
        .expect("child mutex poisoned")
        .take()
        .expect("child present after spawn");

    let mut handles = ChildHandles {
        child,
        stdout_handle,
        stderr_handle,
    };

    tokio::select! {
        biased;
        _ = notify_or_pending(ctx.manual_bg_signal) => {
            auto_backgroundize_foreground(
                handles,
                command,
                ctx.exec.bg_manager,
                true,
                ctx.exec.session_id,
            )
            .await
        }
        result = tokio::time::timeout(bg_timeout, handles.child.wait()) => match result {
            Ok(Ok(status)) => {
                let ChildHandles {
                    stdout_handle,
                    stderr_handle,
                    ..
                } = handles;
                finalize_foreground_after_wait(
                    status, stdout_handle, stderr_handle,
                    command, ctx.exec.session, ctx.exec.call_id,
                ).await
            }
            Ok(Err(e)) => ForegroundOutcome::Failed(
                format!("failed to wait on command: {}", e)
            ),
            Err(_elapsed) => {
                handle_timeout_expiry(handles, command, ctx).await
            }
        },
    }
}

/// Execute the BashTool call: parse args, check two-level permissions, run command.
pub(super) async fn execute_bash_call(
    deps: &PermDeps,
    bg: &Arc<dyn closeclaw_common::TaskManager>,
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

    let sec_result = analyze_security(command)?;
    match route_trust_level(
        deps,
        ctx,
        command,
        sec_result.trust_level,
        sec_result.reason.clone(),
    )
    .await?
    {
        TrustDecision::Blocked(reason) => {
            return Err(ToolCallError::ExecutionFailed(reason));
        }
        TrustDecision::ApprovalPending(result) => return Ok(result),
        TrustDecision::Trusted => {}
    }

    let cwd = prepare_and_sandbox(ctx, command, &args).await?;

    let session_id = ctx.session_id.as_deref().unwrap_or("");
    let exec_ctx = BashExecCtx {
        command,
        cwd: &cwd,
        bg_manager: bg,
        session: ctx.session.as_ref(),
        call_id: ctx.call_id.as_deref(),
        session_id,
    };
    execute_command(
        &exec_ctx,
        args.get("run_in_background") == Some(&Value::Bool(true)),
        ctx.manual_background_signal.as_ref(),
        parse_timeout(&args),
    )
    .await
    .map_err(ToolCallError::ExecutionFailed)
}

/// Spawn a monitor task that polls a background task for terminal state.
///
/// On terminal state, sets the final tool state (which immediately
/// removes the entry from the tracking map). Shared by the
/// explicit-background path and the auto-background path to avoid
/// code duplication.
fn spawn_bg_monitor(
    session: &Arc<dyn closeclaw_common::tool_session::ToolSession>,
    call_id: &str,
    task_id: String,
    bg_manager: &Arc<dyn closeclaw_common::TaskManager>,
) {
    let bg = Arc::clone(bg_manager);
    let s = Arc::clone(session);
    let cid = call_id.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            match bg.get_task(&task_id).await {
                Some(bt) => match bt.state {
                    closeclaw_common::TaskState::Completed { .. } => {
                        s.update_tool_state(&cid, ToolExecState::Completed).await;
                        return;
                    }
                    closeclaw_common::TaskState::Failed { .. } => {
                        s.update_tool_state(&cid, ToolExecState::Failed).await;
                        return;
                    }
                    closeclaw_common::TaskState::Killed => {
                        s.update_tool_state(&cid, ToolExecState::Terminated).await;
                        return;
                    }
                    closeclaw_common::TaskState::Running { .. } => {
                        // Still running — continue polling.
                    }
                },
                None => {
                    // Task removed from manager (cleanup or unknown).
                    // Entry already removed by terminal update_tool_state,
                    // or will be cleaned up by the stop path.
                    return;
                }
            }
        }
    });
}

/// Execution context shared along the bash command execution chain.
///
/// Groups the parameters that flow unchanged through `execute_command`,
/// `execute_foreground_command`, and `execute_background_command`, so
/// those signatures stay within the CONTRIBUTING 6-parameter cap.
pub(crate) struct BashExecCtx<'a> {
    pub(crate) command: &'a str,
    pub(crate) cwd: &'a str,
    pub(crate) bg_manager: &'a Arc<dyn closeclaw_common::TaskManager>,
    pub(crate) session: Option<&'a Arc<dyn closeclaw_common::tool_session::ToolSession>>,
    pub(crate) call_id: Option<&'a str>,
    pub(crate) session_id: &'a str,
}

/// Execute a shell command in the background path.
///
/// Registers the tool call, spawns the background task, registers the
/// kill handle, transitions to `RunningBackground`, and spawns a monitor
/// for terminal-state detection.
async fn execute_background_command(ctx: &BashExecCtx<'_>) -> Result<ToolResult, String> {
    let BashExecCtx {
        command,
        cwd,
        bg_manager,
        session,
        call_id,
        session_id,
    } = *ctx;
    let mut registered_call_id = None;
    if let (Some(s), Some(cid)) = (session, call_id) {
        let summary = truncate_summary(command);
        s.register_tool_call(cid.to_string(), "bash".to_string(), summary)
            .await;
        registered_call_id = Some(cid.to_string());
    }
    // Per #762 design: `spawn_task()` is the "self-cold-start" path.
    let task = bg_manager
        .spawn_task(command, Path::new(cwd), false, session_id)
        .await
        .map_err(|e| {
            if let (Some(s), Some(cid)) = (session, registered_call_id.as_deref()) {
                let s = Arc::clone(s);
                let cid = cid.to_string();
                tokio::spawn(async move {
                    s.update_tool_state(&cid, ToolExecState::Failed).await;
                });
            }
            format!("failed to spawn background task: {}", e)
        })?;
    // Register BackgroundKillHandle so cascade-stop can find the task.
    if let (Some(s), Some(cid)) = (session, call_id) {
        let handle: Arc<dyn closeclaw_common::tool_session::KillHandle> =
            Arc::new(BackgroundKillHandle {
                bg_manager: Arc::clone(bg_manager),
                task_id: task.id.clone(),
            });
        s.register_tool_handle(cid.to_string(), handle).await;
    }
    // Transition to RunningBackground: retain entry for exec_status().
    if let (Some(s), Some(cid)) = (session, call_id) {
        s.update_tool_state(cid, ToolExecState::RunningBackground)
            .await;
        if let Err(e) = s.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %cid,
                "bash background: checkpoint persist failed: {}",
                e
            );
        }
        spawn_bg_monitor(s, cid, task.id.clone(), bg_manager);
    }
    Ok(build_background_result(&task))
}

/// Resolve the foreground wait timeout and force-terminate flag.
///
/// Whitelist + no agent timeout → total-execution-time fallback.
/// All other paths → agent timeout (clamped) or default 15s.
fn resolve_bg_timeout_and_force_terminate(
    command: &str,
    agent_timeout_ms: Option<u64>,
    max_execution_secs: u64,
) -> (Duration, bool) {
    if auto_backgroundize_excluded(command) && agent_timeout_ms.is_none() {
        (Duration::from_secs(max_execution_secs), true)
    } else {
        (
            Duration::from_millis(
                agent_timeout_ms
                    .map(|ms| ms.min(AUTO_BG_TIMEOUT_CAP_MS))
                    .unwrap_or(AUTO_BG_TIMEOUT_MS),
            ),
            false,
        )
    }
}

/// Execute a foreground command and return its [`ForegroundOutcome`].
///
/// Registers the tool call, spawns the child, registers the kill handle,
/// waits for completion, and returns the outcome. Caller is responsible
/// for setting the final tool state and deregistering.
///
/// `manual_bg_signal` is the agent's manual backgroundize signal
/// (`None` disables the branch): firing it mid-wait hands the child to
/// the task manager and returns a user-backgrounded result.
/// `agent_timeout_ms` is the agent-requested timeout budget, clamped to
/// [`AUTO_BG_TIMEOUT_CAP_MS`]; when absent it falls back to
/// [`AUTO_BG_TIMEOUT_MS`], except that auto-backgroundize-excluded
/// commands use the total-execution-time cap with forced termination.
pub(crate) async fn execute_foreground_command(
    ctx: &BashExecCtx<'_>,
    manual_bg_signal: Option<&Arc<tokio::sync::Notify>>,
    agent_timeout_ms: Option<u64>,
) -> Result<(ForegroundOutcome, Option<String>), String> {
    let BashExecCtx {
        command,
        cwd,
        bg_manager,
        session,
        call_id,
        ..
    } = *ctx;
    let child = spawn_sh_command(command, cwd)?;
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));

    let registered_call_id = if let (Some(s), Some(cid)) = (session, call_id) {
        Some(register_foreground_session(s, cid, command, &child_arc).await)
    } else {
        None
    };

    let (bg_timeout, force_terminate) = resolve_bg_timeout_and_force_terminate(
        command,
        agent_timeout_ms,
        bg_manager.max_execution_secs(),
    );

    let fg_ctx = ForegroundContext {
        exec: ctx,
        manual_bg_signal,
        force_terminate,
    };
    let outcome = handle_foreground_result(child_arc, command, bg_timeout, &fg_ctx).await;

    Ok((outcome, registered_call_id))
}

/// Execute a shell command via `sh -c` with timeout.
///
/// When `run_in_background` is true, immediately spawns a background
/// task. Otherwise executes in foreground with a 15-second
/// auto-backgroundize budget.
///
/// `session` and `call_id` (when both `Some`) drive the kill-handle
/// integration added by issue #858: the foreground path
/// registers a [`BashKillHandle`] for the duration of the wait, the
/// background path registers a [`BackgroundKillHandle`] for the
/// lifetime of the task. Both are `None`-safe - tool invocations
/// outside a tracked session (CLI, tests, prompt generation) skip
/// registration entirely.
///
/// On the foreground path `manual_bg_signal` (manual backgroundize
/// signal, `None` disables the branch) and `agent_timeout_ms`
/// (agent-requested budget, clamped/fallback per
/// [`execute_foreground_command`]) are forwarded unchanged; the
/// background path ignores both.
pub(crate) async fn execute_command(
    ctx: &BashExecCtx<'_>,
    run_in_background: bool,
    manual_bg_signal: Option<&Arc<tokio::sync::Notify>>,
    agent_timeout_ms: Option<u64>,
) -> Result<ToolResult, String> {
    if run_in_background {
        return execute_background_command(ctx).await;
    }

    let (outcome, registered_call_id) =
        execute_foreground_command(ctx, manual_bg_signal, agent_timeout_ms).await?;
    match outcome {
        ForegroundOutcome::Completed(result) => {
            if let (Some(s), Some(cid)) = (ctx.session, registered_call_id.as_deref()) {
                s.update_tool_state(cid, ToolExecState::Completed).await;
            }
            Ok(result)
        }
        ForegroundOutcome::Failed(e) => {
            if let (Some(s), Some(cid)) = (ctx.session, registered_call_id.as_deref()) {
                s.update_tool_state(cid, ToolExecState::Failed).await;
            }
            Err(e)
        }
        ForegroundOutcome::AutoBackground(result, task_id) => {
            if let (Some(s), Some(cid)) = (ctx.session, registered_call_id.as_deref()) {
                s.update_tool_state(cid, ToolExecState::RunningBackground)
                    .await;
                if let Err(e) = s.persist_pending_checkpoint().await {
                    tracing::warn!(
                        session_id = %cid,
                        "bash auto-background: checkpoint persist failed: {}",
                        e
                    );
                }
                spawn_bg_monitor(s, cid, task_id, ctx.bg_manager);
            }
            Ok(result)
        }
    }
}
