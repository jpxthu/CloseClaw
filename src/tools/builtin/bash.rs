//! Built-in BashTool — provides shell command execution capability for agents.
//! Implements timeout control, output truncation with head-preservation,
//! output persistence, and command classification.

use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::PermissionResponse;
use crate::tools::builtin::bash_classify;
use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::Value;
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
}

impl BashTool {
    /// Creates a new `BashTool` backed by the given permission engine.
    pub fn new(permission_engine: Arc<PermissionEngine>) -> Self {
        Self { permission_engine }
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
         strategy (threshold 30,000 chars), and output persistence to disk \
         when output exceeds threshold. Returns stdout, stderr, exit code, \
         and semantic interpretation of return codes for known command types."
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
                    "description": "Run command in background (not yet implemented)"
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

        if args.get("run_in_background") == Some(&Value::Bool(true)) {
            return Err(ToolCallError::ExecutionFailed(
                "background tasks not yet implemented".into(),
            ));
        }
        // `description` and `dangerouslyDisableSandbox` are parsed but ignored.
        let _ = args.get("description");
        let _ = args.get("dangerouslyDisableSandbox");

        // --- 2. Permission check ---
        if let PermissionResponse::Denied { reason, .. } =
            self.permission_engine.check(&ctx.agent_id, "exec")
        {
            return Err(ToolCallError::PermissionDenied(reason));
        }

        // --- 3. Execute subprocess ---
        let result = execute_command(command, &cwd, timeout_ms).await;
        match result {
            Ok(r) => Ok(r),
            Err(e) => Err(ToolCallError::ExecutionFailed(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

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

/// Execute a shell command via `sh -c` with timeout.
///
/// Returns a [`ToolResult`] on success or an error message on failure.
async fn execute_command(command: &str, cwd: &str, timeout_ms: u64) -> Result<ToolResult, String> {
    use tokio::process::Command;

    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn command: {}", e))?;

    let output =
        tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await;

    match output {
        Ok(Ok(output)) => {
            let stdout_raw = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr_raw = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout_p = process_output(&stdout_raw);
            let stderr_p = process_output(&stderr_raw);
            Ok(build_result(command, stdout_p, stderr_p, exit_code, false))
        }
        Ok(Err(e)) => Err(format!("failed to wait on command: {}", e)),
        Err(_elapsed) => Ok(build_result(
            command,
            process_output(""),
            process_output(""),
            -1,
            true,
        )),
    }
}

/// Truncate a string at a safe UTF-8 character boundary.
/// Returns the first `max_bytes` bytes without splitting a multi-byte character.
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

/// Truncate output beyond [`MAX_OUTPUT_CHARS`] and persist full output to disk.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_permission_engine() -> Arc<PermissionEngine> {
        use crate::permission::rules::RuleSetBuilder;
        Arc::new(PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new().build().unwrap(),
        ))
    }

    fn test_tool_context() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
        }
    }

    // --- process_output ---

    #[test]
    fn test_process_output_short_string() {
        let out = process_output("hello world");
        assert_eq!(out.inline, "hello world");
        assert!(out.persisted_path.is_none());
        assert_eq!(out.persisted_size, 0);
    }

    #[test]
    fn test_process_output_exact_boundary() {
        let exact: String = "a".repeat(MAX_OUTPUT_CHARS);
        let out = process_output(&exact);
        assert_eq!(out.inline, exact);
        assert!(out.persisted_path.is_none());
    }

    #[test]
    fn test_process_output_long_string_truncates() {
        let long: String = "x".repeat(MAX_OUTPUT_CHARS + 1000);
        let out = process_output(&long);
        // When persist succeeds, inline is a <persisted-output> reference
        assert!(out.inline.contains("<persisted-output"));
        assert!(out.persisted_path.is_some());
        assert_eq!(out.persisted_size, long.len());
        if let Some(ref p) = out.persisted_path {
            let _ = std::fs::remove_file(p);
        }
    }

    // --- persist_output ---

    #[test]
    fn test_persist_output_writes_file() {
        let path = persist_output("test persist data").unwrap();
        assert!(std::path::Path::new(&path).exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "test persist data");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_output_cleans_up() {
        let path = persist_output("cleanup test").unwrap();
        assert!(std::path::Path::new(&path).exists());
        std::fs::remove_file(&path).unwrap();
        assert!(!std::path::Path::new(&path).exists());
    }

    // --- parse_timeout ---

    #[test]
    fn test_parse_timeout_default() {
        let args = serde_json::json!({});
        assert_eq!(parse_timeout(&args), 120_000);
    }

    #[test]
    fn test_parse_timeout_custom() {
        let args = serde_json::json!({"timeout": 5000});
        assert_eq!(parse_timeout(&args), 5000);
    }

    #[test]
    fn test_parse_timeout_clamped() {
        let args = serde_json::json!({"timeout": 900_000});
        assert_eq!(parse_timeout(&args), 600_000);
    }

    #[test]
    fn test_parse_timeout_zero() {
        let args = serde_json::json!({"timeout": 0});
        assert_eq!(parse_timeout(&args), 0);
    }

    // --- resolve_cwd ---

    #[test]
    fn test_resolve_cwd_no_cwd_no_workdir() {
        let args = serde_json::json!({});
        let ctx = test_tool_context();
        let result = resolve_cwd(&args, &ctx);
        let expected = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());
        assert_eq!(result, expected);
    }

    #[test]
    fn test_resolve_cwd_with_cwd_arg() {
        let args = serde_json::json!({"cwd": "/tmp/test"});
        let ctx = test_tool_context();
        assert_eq!(resolve_cwd(&args, &ctx), "/tmp/test");
    }

    // --- BashTool metadata ---

    #[test]
    fn test_bash_tool_name_and_group() {
        let tool = BashTool::new(test_permission_engine());
        assert_eq!(tool.name(), "Bash");
        assert_eq!(tool.group(), "bash");
    }

    #[test]
    fn test_bash_tool_flags() {
        let tool = BashTool::new(test_permission_engine());
        let flags = tool.flags();
        assert!(flags.is_destructive);
        assert!(flags.is_expensive);
    }

    // --- input_schema ---

    #[test]
    fn test_input_schema_command_required() {
        let tool = BashTool::new(test_permission_engine());
        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("command")));
    }

    #[test]
    fn test_input_schema_six_properties() {
        let tool = BashTool::new(test_permission_engine());
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert_eq!(props.len(), 6);
        for name in &[
            "command",
            "timeout",
            "description",
            "run_in_background",
            "cwd",
            "dangerouslyDisableSandbox",
        ] {
            assert!(props.contains_key(*name), "missing property: {}", name);
        }
    }
}
