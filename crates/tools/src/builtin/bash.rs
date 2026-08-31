//! Built-in BashTool - provides shell command execution capability for agents.
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
use crate::security::{BashSecurityAnalyzer, ParseResult, SimpleCommand, TrustLevel};
use crate::{PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use closeclaw_common::ToolExecState;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{Caller, PermissionRequestBody};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;

use super::bash_kill::{
    build_auto_background_result, build_background_result, build_manual_background_result,
    finalize_foreground_after_wait, register_foreground_session, truncate_summary,
    BackgroundKillHandle,
};

/// Outcome of foreground tool execution, distinguishing between
/// normal completion and auto-backgroundizing on timeout.
///
/// Used by [`execute_command`] to determine whether to set
/// `Completed`/`Failed` (and deregister) or `RunningBackground`
/// (and retain the tool state entry) after the foreground wait.
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
const AUTO_BG_TIMEOUT_MS: u64 = 15_000;

/// Maximum auto-backgroundize timeout an agent may request (2 minutes).
/// Prevents agents from setting excessively long timeouts that would
/// defeat the auto-backgroundize mechanism.
const AUTO_BG_TIMEOUT_CAP_MS: u64 = 120_000;

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
         Background tasks notify automatically on completion - do not poll. \
         Use run_in_background for commands expected to exceed 10 seconds."
            .to_string()
    }

    fn generate_prompt(&self, context: &PromptGenerationContext) -> String {
        let mut parts = Vec::new();

        parts.push(prompt_when_to_use());
        prompt_add_workdir_guidance(context, &mut parts);
        prompt_add_permission_status(context, &mut parts);
        parts.push(prompt_usage_principles());
        prompt_add_combination_suggestions(context, &mut parts);

        parts.join("\n\n")
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

// --- Prompt generation helpers ---

/// "When to use" section for the Bash tool prompt.
fn prompt_when_to_use() -> String {
    "Use Bash to execute shell commands, run scripts, compile code, \
     or run tests. Supports timeout control (default 120s, max 600s), \
     output truncation with head-preservation (threshold 30,000 chars), \
     and output persistence to disk when output exceeds threshold. \
     Commands exceeding 15s are auto-backgrounded. Background tasks \
     notify automatically on completion — do not poll. \
     Use run_in_background for commands expected to exceed 10 seconds."
        .to_string()
}

/// Append workdir-based path guidance to the prompt parts.
fn prompt_add_workdir_guidance(context: &PromptGenerationContext, parts: &mut Vec<String>) {
    if let Some(ref wd) = context.workdir {
        parts.push(closeclaw_common::format_workdir_guidance(
            wd,
            "Commands run here by default unless the cwd parameter is specified.",
        ));
    }
}

/// Append permission-awareness text to the prompt parts.
///
/// Uses `available_tool_names` (the runtime-computed list after
/// whitelist + blacklist filtering) when available, falling back to
/// the `tools` whitelist for backward compatibility.
fn prompt_add_permission_status(context: &PromptGenerationContext, parts: &mut Vec<String>) {
    let has_bash_access = if !context.available_tool_names.is_empty() {
        context
            .available_tool_names
            .iter()
            .any(|t| t.eq_ignore_ascii_case("Bash"))
    } else {
        context.tools.as_ref().is_none_or(|tools| {
            tools
                .iter()
                .any(|t| t.eq_ignore_ascii_case("Bash") || t == "*")
        })
    };
    if has_bash_access {
        parts.push(
            "Bash is available for your use. Commands are subject to \
             permission checks — untrusted commands may be sandboxed \
             or routed to the approval flow."
                .to_string(),
        );
    } else {
        parts.push(
            "Bash is not available in your current tool set. \
             You do not have permission to execute shell commands."
                .to_string(),
        );
    }
}

/// "Usage principles" section for the Bash tool prompt.
fn prompt_usage_principles() -> String {
    "Keep commands concise and focused. Prefer specific commands \
     over complex pipelines when possible. Avoid long-running \
     foreground commands — use run_in_background for tasks \
     expected to exceed 10 seconds."
        .to_string()
}

/// Append combination suggestions based on available tools.
fn prompt_add_combination_suggestions(context: &PromptGenerationContext, parts: &mut Vec<String>) {
    let has_read = context
        .available_tool_names
        .iter()
        .any(|t| t == "Read" || t == "read");
    let has_write = context
        .available_tool_names
        .iter()
        .any(|t| t == "Write" || t == "write");
    if has_read || has_write {
        let mut suggestions = Vec::new();
        if has_read {
            suggestions.push("Read (read files before editing)");
        }
        if has_write {
            suggestions.push("Write (redirect output to files)");
        }
        parts.push(format!("Combine with: {}.", suggestions.join(", ")));
    }
}

// --- Helper functions ---

/// Parse and clamp the agent-specified timeout parameter.
///
/// Returns `None` when no timeout is provided.
/// Max 600 000 ms.
fn parse_timeout(args: &Value) -> Option<u64> {
    let raw = args.get("timeout").and_then(Value::as_f64)?;
    let ms = raw.max(0.0) as u64;
    Some(ms.min(600_000))
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
) -> Result<(ToolResult, String), String> {
    child.stdout = stdout_handle;
    child.stderr = stderr_handle;
    let task = bg_manager
        .backgroundize_task(child, command, true)
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
async fn auto_backgroundize_foreground(
    child: tokio::process::Child,
    stdout_handle: Option<tokio::process::ChildStdout>,
    stderr_handle: Option<tokio::process::ChildStderr>,
    command: &str,
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
    by_user: bool,
) -> ForegroundOutcome {
    match backgroundize_child(
        child,
        stdout_handle,
        stderr_handle,
        command,
        bg_manager,
        by_user,
    )
    .await
    {
        Ok((result, task_id)) => ForegroundOutcome::AutoBackground(result, task_id),
        Err(e) => ForegroundOutcome::Failed(e),
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
    session: Option<&Arc<dyn closeclaw_common::tool_session::ToolSession>>,
    call_id: Option<&str>,
) -> ForegroundOutcome {
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
            auto_backgroundize_foreground(
                child, stdout_handle, stderr_handle, command, bg_manager, true,
            ).await
        }
        result = tokio::time::timeout(bg_timeout, child.wait()) => match result {
            Ok(Ok(status)) => {
                finalize_foreground_after_wait(
                    status, stdout_handle, stderr_handle,
                    command, session, call_id,
                ).await
            }
            Ok(Err(e)) => ForegroundOutcome::Failed(
                format!("failed to wait on command: {}", e)
            ),
            Err(_elapsed) => {
                auto_backgroundize_foreground(
                    child, stdout_handle, stderr_handle,
                    command, bg_manager, false,
                ).await
            }
        },
    }
}

/// Analyze command security and return the parsed result.
///
/// Returns the [`ParseResult`] so the caller can inspect `trust_level`
/// and use the tree-sitter parsed `commands` (argv arrays) for
/// downstream permission checks.
fn analyze_security(command: &str) -> Result<ParseResult, ToolCallError> {
    let sec_result = BashSecurityAnalyzer::new()
        .map_err(ToolCallError::ExecutionFailed)?
        .analyze(command);
    Ok(sec_result)
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
    match check_command_permission(deps, ctx, cmd_name, cmd_args, None).await {
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

/// Submit a trust-level denial to the approval flow and optionally notify the owner.
///
/// Builds a [`Caller`] and [`PermissionRequestBody`] from the tool context, then
/// calls `submit_denial()` on the approval flow. Returns the request ID if the
/// approval flow accepted the request (i.e. owner was notified / queued), or
/// `None` if hard-denied (sub-agent or duplicate).
///
/// P3 note: `user_id` is intentionally left empty here because
/// trust-level routing (malicious/uncertain) occurs before user identity
/// verification — at this point in the pipeline we only know the agent ID,
/// not the Feishu user identity. The approval flow will resolve the actual
/// user identity from the session manager when it processes the request.
async fn submit_trust_level_denial(
    deps: &PermDeps,
    ctx: &ToolContext,
    command: &str,
    risk: RiskLevel,
) -> Option<String> {
    let (_, session_mgr, _, approval_flow) = deps;
    let caller = Caller {
        user_id: String::new(),
        agent: ctx.agent_id.clone(),
    };
    let body = PermissionRequestBody::CommandExec {
        agent: ctx.agent_id.clone(),
        cmd: command.to_string(),
        args: vec![],
    };
    let sid = ctx.session_id.as_deref().unwrap_or("");
    let is_sub_agent = crate::permission_check::is_session_sub_agent(session_mgr, sid).await;
    let mut flow = approval_flow.lock().await;
    let request_id = flow.submit_denial(&caller, &body, risk, sid, is_sub_agent);
    drop(flow);
    request_id
}

/// Execute the BashTool call: parse args, check two-level permissions, run command.
/// Result of trust-level routing.
enum TrustDecision {
    /// Command is trusted; proceed with normal permission checks.
    Trusted,
    /// Command was routed to approval flow; caller returns the pending result.
    ApprovalPending(ToolResult),
    /// Command was blocked (malicious or uncertain-rejected).
    Blocked(String),
}

/// Route trust-level decisions (malicious/uncertain) through the approval flow.
async fn route_trust_level(
    deps: &PermDeps,
    ctx: &ToolContext,
    command: &str,
    trust_level: TrustLevel,
    reason: Option<String>,
) -> Result<TrustDecision, ToolCallError> {
    match trust_level {
        TrustLevel::Malicious => {
            submit_trust_level_denial(deps, ctx, command, RiskLevel::Critical).await;
            Ok(TrustDecision::Blocked(format!(
                "Blocked: malicious command detected — {}",
                reason.unwrap_or_else(|| "unknown reason".into())
            )))
        }
        TrustLevel::Uncertain => {
            let r = reason.unwrap_or_else(|| "untrusted syntax".into());
            if let Some(request_id) =
                submit_trust_level_denial(deps, ctx, command, RiskLevel::High).await
            {
                Ok(TrustDecision::ApprovalPending(ToolResult {
                    data: crate::builtin::approval_utils::build_approval_pending(request_id),
                    new_messages: vec![],
                    context_modifier: None,
                }))
            } else {
                Ok(TrustDecision::Blocked(format!(
                    "Blocked: uncertain command — {}",
                    r
                )))
            }
        }
        TrustLevel::Trusted => Ok(TrustDecision::Trusted),
    }
}

/// Extract command name and argv from the parse result.
fn extract_argv(command: &str, commands: &[SimpleCommand]) -> (String, Vec<String>) {
    commands
        .first()
        .map(|cmd| {
            let name = cmd.argv.first().cloned().unwrap_or_else(|| "*".into());
            let args = cmd.argv[1..].to_vec();
            (name, args)
        })
        .unwrap_or_else(|| {
            let cmd_parts: Vec<&str> = command.split_whitespace().collect();
            let name = cmd_parts.first().copied().unwrap_or("*").to_string();
            let args = cmd_parts[1..].iter().map(|s| s.to_string()).collect();
            (name, args)
        })
}

/// Analyze security, extract argv, check Level 2 permission, and apply sandbox.
///
/// Returns `(Ok(Some(ToolResult)), _)` if routed to approval,
/// or `(Ok(None), cwd)` to proceed with normal execution.
async fn prepare_and_sandbox(
    deps: &PermDeps,
    ctx: &ToolContext,
    command: &str,
    args: &Value,
) -> Result<(Option<ToolResult>, String), ToolCallError> {
    let sec_result = analyze_security(command)?;
    let (cmd_name, cmd_args) = extract_argv(command, &sec_result.commands);
    let dangerously_disable = args.get("dangerouslyDisableSandbox") == Some(&Value::Bool(true));
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
        return Ok((Some(r), String::new()));
    }
    let cwd = resolve_cwd(args, ctx);
    if !sandbox_already_applied
        && !dangerously_disable
        && CommandSandbox::should_sandbox(command, true)
    {
        CommandSandbox::apply_sandbox_restrictions(&cwd)?;
    }
    Ok((None, cwd))
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

    // Level 1: ToolCall - verify agent may invoke Bash tool.
    if let Some(r) = check_tool_permission(deps, ctx, "bash", "call", None).await? {
        return Ok(r);
    }

    let (approval_result, cwd) = prepare_and_sandbox(deps, ctx, command, &args).await?;
    if let Some(r) = approval_result {
        return Ok(r);
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
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
) {
    let bg = Arc::clone(bg_manager);
    let s = Arc::clone(session);
    let cid = call_id.to_string();
    tokio::spawn(async move {
        let task_id = task_id;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            match bg.get_task(&task_id).await {
                Some(bt) => match bt.state {
                    closeclaw_tasks::TaskState::Completed { .. } => {
                        s.update_tool_state(&cid, ToolExecState::Completed).await;
                        return;
                    }
                    closeclaw_tasks::TaskState::Failed { .. } => {
                        s.update_tool_state(&cid, ToolExecState::Failed).await;
                        return;
                    }
                    closeclaw_tasks::TaskState::Killed => {
                        s.update_tool_state(&cid, ToolExecState::Terminated).await;
                        return;
                    }
                    closeclaw_tasks::TaskState::Running { .. } => {
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

/// Execute a shell command in the background path.
///
/// Registers the tool call, spawns the background task, registers the
/// kill handle, transitions to `RunningBackground`, and spawns a monitor
/// for terminal-state detection.
async fn execute_background_command(
    command: &str,
    cwd: &str,
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
    session: Option<&Arc<dyn closeclaw_common::tool_session::ToolSession>>,
    call_id: Option<&str>,
) -> Result<ToolResult, String> {
    let mut registered_call_id = None;
    if let (Some(s), Some(cid)) = (session, call_id) {
        let summary = truncate_summary(command);
        s.register_tool_call(cid.to_string(), "bash".to_string(), summary)
            .await;
        registered_call_id = Some(cid.to_string());
    }
    // Per #762 design: `spawn_task()` is the "self-cold-start" path.
    let task = bg_manager
        .spawn_task(command, Path::new(cwd), false)
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

/// Execute a foreground command and return its [`ForegroundOutcome`].
///
/// Registers the tool call, spawns the child, registers the kill handle,
/// waits for completion, and returns the outcome. Caller is responsible
/// for setting the final tool state and deregistering.
#[allow(clippy::too_many_arguments)]
async fn execute_foreground_command(
    command: &str,
    cwd: &str,
    agent_timeout_ms: Option<u64>,
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
    session: Option<&Arc<dyn closeclaw_common::tool_session::ToolSession>>,
    call_id: Option<&str>,
    manual_bg_signal: Option<&Arc<tokio::sync::Notify>>,
) -> Result<(ForegroundOutcome, Option<String>), String> {
    let child = spawn_sh_command(command, cwd)?;
    let child_arc: Arc<Mutex<Option<tokio::process::Child>>> = Arc::new(Mutex::new(Some(child)));

    let registered_call_id = if let (Some(s), Some(cid)) = (session, call_id) {
        Some(register_foreground_session(s, cid, command, &child_arc).await)
    } else {
        None
    };

    let bg_timeout = if auto_backgroundize_excluded(command) {
        Duration::from_millis(AUTO_BG_TIMEOUT_CAP_MS)
    } else {
        Duration::from_millis(
            agent_timeout_ms
                .map(|ms| ms.min(AUTO_BG_TIMEOUT_CAP_MS))
                .unwrap_or(AUTO_BG_TIMEOUT_MS),
        )
    };

    let outcome = handle_foreground_result(
        child_arc,
        command,
        bg_timeout,
        bg_manager,
        manual_bg_signal,
        session,
        call_id,
    )
    .await;

    Ok((outcome, registered_call_id))
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
/// lifetime of the task. Both are `None`-safe - tool invocations
/// outside a tracked session (CLI, tests, prompt generation) skip
/// registration entirely.
#[allow(clippy::too_many_arguments)]
async fn execute_command(
    command: &str,
    cwd: &str,
    agent_timeout_ms: Option<u64>,
    run_in_background: bool,
    bg_manager: &Arc<dyn closeclaw_tasks::TaskManager>,
    session: Option<&Arc<dyn closeclaw_common::tool_session::ToolSession>>,
    call_id: Option<&str>,
    manual_bg_signal: Option<&Arc<tokio::sync::Notify>>,
) -> Result<ToolResult, String> {
    if run_in_background {
        return execute_background_command(command, cwd, bg_manager, session, call_id).await;
    }

    let (outcome, registered_call_id) = execute_foreground_command(
        command,
        cwd,
        agent_timeout_ms,
        bg_manager,
        session,
        call_id,
        manual_bg_signal,
    )
    .await?;
    match outcome {
        ForegroundOutcome::Completed(result) => {
            if let (Some(s), Some(cid)) = (session, registered_call_id.as_deref()) {
                s.update_tool_state(cid, ToolExecState::Completed).await;
            }
            Ok(result)
        }
        ForegroundOutcome::Failed(e) => {
            if let (Some(s), Some(cid)) = (session, registered_call_id.as_deref()) {
                s.update_tool_state(cid, ToolExecState::Failed).await;
            }
            Err(e)
        }
        ForegroundOutcome::AutoBackground(result, task_id) => {
            if let (Some(s), Some(cid)) = (session, registered_call_id.as_deref()) {
                s.update_tool_state(cid, ToolExecState::RunningBackground)
                    .await;
                if let Err(e) = s.persist_pending_checkpoint().await {
                    tracing::warn!(
                        session_id = %cid,
                        "bash auto-background: checkpoint persist failed: {}",
                        e
                    );
                }
                spawn_bg_monitor(s, cid, task_id, bg_manager);
            }
            Ok(result)
        }
    }
}

#[cfg(test)]
#[path = "bash_approval_tests.rs"]
mod approval_tests;

#[cfg(test)]
#[path = "bash_gap_tests.rs"]
mod gap_tests;

#[cfg(test)]
#[path = "bash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bash_timeout_tests.rs"]
mod timeout_tests;
