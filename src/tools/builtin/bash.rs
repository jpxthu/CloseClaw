//! Built-in BashTool — provides shell command execution capability for agents.
//! Implements timeout control, output truncation with head-preservation,
//! output persistence, and command classification.

use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::PermissionResponse;
use crate::tasks::BackgroundTaskManager;
use crate::tools::builtin::bash_classify;
use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Maximum characters per output stream (stdout/stderr) before truncation.
const MAX_OUTPUT_CHARS: usize = 30_000;

/// Maximum bytes for persisted output file (64 MB).
const MAX_PERSISTED_BYTES: usize = 64 * 1024 * 1024;

/// Number of bytes to preview from persisted output.
const PREVIEW_BYTES: usize = 2_000;

/// Directory for persisted output files.
const PERSIST_DIR: &str = "/tmp/openclaw";

/// Auto-backgroundize timeout (15 seconds).
const AUTO_BG_TIMEOUT_MS: u64 = 15_000;

/// Processed output: either inline content or a persisted-output reference.
struct OutputProcessed {
    inline: String,
    persisted_path: Option<String>,
    persisted_size: usize,
}

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

/// Read all bytes from an optional async reader and return as String.
async fn read_pipe<R: tokio::io::AsyncRead + Unpin>(pipe: Option<R>) -> String {
    use tokio::io::AsyncReadExt;
    match pipe {
        Some(mut r) => {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        }
        None => String::new(),
    }
}

/// Execute the BashTool call: parse args, check permissions, run command.
async fn execute_bash_call(
    perm: &PermissionEngine,
    bg: &BackgroundTaskManager,
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

    // --- 2. Permission check ---
    if let PermissionResponse::Denied { reason, .. } = perm.check(&ctx.agent_id, "exec") {
        return Err(ToolCallError::PermissionDenied(reason));
    }

    // --- 3. Execute subprocess ---
    let result = execute_command(command, &cwd, timeout_ms, run_in_background, bg).await;
    match result {
        Ok(r) => Ok(r),
        Err(e) => Err(ToolCallError::ExecutionFailed(e)),
    }
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
/// On success, reads stdout/stderr and builds the tool result.
/// On timeout, hands the child back to the background task manager.
async fn handle_foreground_result(
    mut child: tokio::process::Child,
    command: &str,
    cwd: &str,
    stdout_handle: Option<tokio::process::ChildStdout>,
    stderr_handle: Option<tokio::process::ChildStderr>,
    bg_timeout: Duration,
    bg_manager: &BackgroundTaskManager,
) -> Result<ToolResult, String> {
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
                .backgroundize(child, command, Path::new(cwd))
                .await
                .map_err(|e| format!("failed to backgroundize command: {}", e))?;
            Ok(build_auto_background_result(&task))
        }
    }
}

/// Execute a shell command via `sh -c` with timeout.
///
/// When `run_in_background` is true, immediately spawns a background
/// task. Otherwise executes in foreground with a 15-second
/// auto-backgroundize budget.
async fn execute_command(
    command: &str,
    cwd: &str,
    timeout_ms: u64,
    run_in_background: bool,
    bg_manager: &BackgroundTaskManager,
) -> Result<ToolResult, String> {
    let mut child = spawn_sh_command(command, cwd)?;

    if run_in_background {
        let task = bg_manager
            .backgroundize(child, command, Path::new(cwd))
            .await
            .map_err(|e| format!("failed to background task: {}", e))?;
        return Ok(build_background_result(&task));
    }

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let bg_timeout = if auto_backgroundize_excluded(command) {
        Duration::from_millis(timeout_ms)
    } else {
        Duration::from_millis(AUTO_BG_TIMEOUT_MS)
    };

    handle_foreground_result(
        child,
        command,
        cwd,
        stdout_handle,
        stderr_handle,
        bg_timeout,
        bg_manager,
    )
    .await
}

/// Truncate a string at a safe UTF-8 character boundary.
/// Returns the first `max_bytes` bytes without splitting a multi-byte
/// character.
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
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

/// Truncate output beyond [`MAX_OUTPUT_CHARS`] and persist full output
/// to disk.
///
/// Returns an [`OutputProcessed`] with either inline content or a
/// persisted-output reference string.
fn process_output(raw: &str) -> OutputProcessed {
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

/// Persist full output to a temporary file.
///
/// Writes to `/tmp/openclaw/bash_output_{ts}_{pid}_{counter}.txt`. If the
/// output exceeds [`MAX_PERSISTED_BYTES`], the file is truncated.
fn persist_output(raw: &str) -> Result<String, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
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

/// Build a [`ToolResult`] from processed execution outputs.
fn build_result(
    command: &str,
    stdout: OutputProcessed,
    stderr: OutputProcessed,
    exit_code: i32,
    interrupted: bool,
) -> ToolResult {
    let category = bash_classify::classify_command(command);
    let no_output = bash_classify::no_output_expected(category);
    let interpretation = bash_classify::interpret_exit_code(command, exit_code);
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
