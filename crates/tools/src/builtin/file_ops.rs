//! Built-in tools — file operations (Tool trait implementation).
//!
//! Each tool is an independent [`Tool`] implementation, completely separate
//! from the [`crate::skills`] module.  All five tools share two-level
//! permission checks via [`crate::permission_check`].

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;

use crate::permission_check;
use crate::permission_check::PermDeps;
use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

type PermEngine = Arc<tokio::sync::RwLock<PermissionEngine>>;
type SessionMgr = Arc<SessionManager>;
type ConfigMgr = Arc<ConfigManager>;
type ApprovalMtx = Arc<tokio::sync::Mutex<ApprovalFlow>>;

// ---------------------------------------------------------------------------
// Shared two-level permission check + I/O dispatch
// ---------------------------------------------------------------------------

/// Two-level permission check then execute `io_fn`.
///
/// Level 1: ToolCall dimension — agent must be allowed to invoke the tool.
/// Level 2: FileOp dimension — agent must have read/write access to the path.
/// On denial, routes through [`ApprovalFlow`].
async fn check_and_execute<F>(
    deps: &PermDeps,
    ctx: &ToolContext,
    path: &str,
    op: &str,
    io_fn: F,
) -> Result<ToolResult, ToolCallError>
where
    F: std::future::Future<Output = Result<ToolResult, ToolCallError>>,
{
    if let Some(r) = permission_check::check_tool_permission(deps, ctx, "file_ops", "call").await? {
        return Ok(r);
    }
    if let Some(r) = permission_check::check_file_op_permission(deps, ctx, path, op).await? {
        return Ok(r);
    }
    if op == "write" && permission_check::is_config_file(deps.2.as_ref(), path) {
        if let Some(r) = permission_check::check_config_write_permission(deps, ctx, path).await? {
            return Ok(r);
        }
    }
    io_fn.await
}

/// Extract a required string argument from `args`, returning an error if missing.
fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolCallError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolCallError::InvalidArgs(format!("missing required parameter: {key}")))
}

/// Read a file and return its content as JSON.
async fn read_file(path: &str) -> Result<ToolResult, ToolCallError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ToolCallError::ExecutionFailed(format!("{path}: {e}")))?;
    Ok(ToolResult {
        data: serde_json::json!({ "content": content }),
        new_messages: vec![],
        context_modifier: None,
    })
}

/// Write content to a file, creating parent directories as needed.
async fn write_file(path: &str, content: &str) -> Result<ToolResult, ToolCallError> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ToolCallError::ExecutionFailed(format!("create_dir_all: {e}")))?;
    }
    std::fs::write(path, content)
        .map_err(|e| ToolCallError::ExecutionFailed(format!("{path}: {e}")))?;
    Ok(ToolResult {
        data: serde_json::json!({ "content": content }),
        new_messages: vec![],
        context_modifier: None,
    })
}

/// Apply a targeted text replacement in a file.
async fn edit_file(path: &str, old: &str, new: &str) -> Result<ToolResult, ToolCallError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ToolCallError::ExecutionFailed(format!("{path}: {e}")))?;
    if !content.contains(old) {
        return Err(ToolCallError::ExecutionFailed(format!(
            "oldText not found verbatim in {path}"
        )));
    }
    let updated = content.replacen(old, new, 1);
    std::fs::write(path, &updated)
        .map_err(|e| ToolCallError::ExecutionFailed(format!("{path}: {e}")))?;
    Ok(ToolResult {
        data: serde_json::json!({ "content": updated }),
        new_messages: vec![],
        context_modifier: None,
    })
}

/// Recursively grep for pattern matches in a directory.
fn grep_walk(dir: &Path, re: &Regex, results: &mut Vec<Value>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut sorted: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    sorted.sort_by_key(|e| e.file_name());
    for entry in sorted {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let p = entry.path();
        if ft.is_dir() {
            grep_walk(&p, re, results);
            continue;
        }
        grep_file(&p, re, results);
    }
}

/// Grep a single file for pattern matches.
fn grep_file(path: &Path, re: &Regex, results: &mut Vec<Value>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for (i, line) in content.lines().enumerate() {
        if re.is_match(line) {
            results.push(serde_json::json!({
                "file": path.to_string_lossy(),
                "line_number": i + 1,
                "line": line,
            }));
        }
    }
}

/// List directory entries as a JSON array.
async fn list_dir(path: &str) -> Result<ToolResult, ToolCallError> {
    let entries: Vec<String> = std::fs::read_dir(path)
        .map_err(|e| ToolCallError::ExecutionFailed(format!("{path}: {e}")))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    Ok(ToolResult {
        data: serde_json::json!({ "entries": entries }),
        new_messages: vec![],
        context_modifier: None,
    })
}

// ---------------------------------------------------------------------------
// ReadTool
// ---------------------------------------------------------------------------

pub struct ReadTool {
    permission_engine: PermEngine,
    session_manager: SessionMgr,
    config_manager: ConfigMgr,
    approval_flow: ApprovalMtx,
}

impl ReadTool {
    pub fn new(perm: PermEngine, sm: SessionMgr, cm: ConfigMgr, af: ApprovalMtx) -> Self {
        Self {
            permission_engine: perm,
            session_manager: sm,
            config_manager: cm,
            approval_flow: af,
        }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }
    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Read file contents".to_string()
    }

    fn detail(&self) -> String {
        "Read the full contents of a file given its path.\
         Returns the text content as a JSON object with key `content`.\
         Fails if the path does not exist or is not a readable file."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workdir-relative file path"
                }
            },
            "required": ["path"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let path = required_str(&args, "path")?;
        let deps = (
            self.permission_engine.clone(),
            self.session_manager.clone(),
            self.config_manager.clone(),
            self.approval_flow.clone(),
        );
        check_and_execute(&deps, ctx, path, "read", read_file(path)).await
    }
}

// ---------------------------------------------------------------------------
// WriteTool
// ---------------------------------------------------------------------------

pub struct WriteTool {
    permission_engine: PermEngine,
    session_manager: SessionMgr,
    config_manager: ConfigMgr,
    approval_flow: ApprovalMtx,
}

impl WriteTool {
    pub fn new(perm: PermEngine, sm: SessionMgr, cm: ConfigMgr, af: ApprovalMtx) -> Self {
        Self {
            permission_engine: perm,
            session_manager: sm,
            config_manager: cm,
            approval_flow: af,
        }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }
    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Write content to a file".to_string()
    }

    fn detail(&self) -> String {
        "Write text content to a file, creating it or overwriting it.\
         Takes `path` (string) and `content` (string).\
         Parent directories are created automatically.\
         Destructive: will overwrite existing files without warning."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workdir-relative file path"
                },
                "content": {
                    "type": "string",
                    "description": "Text content to write"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_read_only: false,
            is_destructive: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let path = required_str(&args, "path")?;
        let content = required_str(&args, "content")?;
        let deps = (
            self.permission_engine.clone(),
            self.session_manager.clone(),
            self.config_manager.clone(),
            self.approval_flow.clone(),
        );
        check_and_execute(&deps, ctx, path, "write", write_file(path, content)).await
    }
}

// ---------------------------------------------------------------------------
// EditTool
// ---------------------------------------------------------------------------

pub struct EditTool {
    permission_engine: PermEngine,
    session_manager: SessionMgr,
    config_manager: ConfigMgr,
    approval_flow: ApprovalMtx,
}

impl EditTool {
    pub fn new(perm: PermEngine, sm: SessionMgr, cm: ConfigMgr, af: ApprovalMtx) -> Self {
        Self {
            permission_engine: perm,
            session_manager: sm,
            config_manager: cm,
            approval_flow: af,
        }
    }
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }
    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Apply a targeted edit to a file".to_string()
    }

    fn detail(&self) -> String {
        "Apply a targeted edit to an existing file using exact text replacement.\
         Takes `path`, `oldText` (exact string to replace), and `newText` (replacement).\
         Only one replacement is performed per call.\
         Fails if `oldText` is not found verbatim in the file.\
         Destructive: modifies the file in place."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workdir-relative file path"
                },
                "oldText": {
                    "type": "string",
                    "description": "Exact text to search for and replace"
                },
                "newText": {
                    "type": "string",
                    "description": "Replacement text"
                }
            },
            "required": ["path", "oldText", "newText"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_read_only: false,
            is_destructive: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let path = required_str(&args, "path")?;
        let old_text = required_str(&args, "oldText")?;
        let new_text = args.get("newText").and_then(Value::as_str).unwrap_or("");
        let deps = (
            self.permission_engine.clone(),
            self.session_manager.clone(),
            self.config_manager.clone(),
            self.approval_flow.clone(),
        );
        check_and_execute(
            &deps,
            ctx,
            path,
            "write",
            edit_file(path, old_text, new_text),
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// GrepTool
// ---------------------------------------------------------------------------

pub struct GrepTool {
    permission_engine: PermEngine,
    session_manager: SessionMgr,
    config_manager: ConfigMgr,
    approval_flow: ApprovalMtx,
}

impl GrepTool {
    pub fn new(perm: PermEngine, sm: SessionMgr, cm: ConfigMgr, af: ApprovalMtx) -> Self {
        Self {
            permission_engine: perm,
            session_manager: sm,
            config_manager: cm,
            approval_flow: af,
        }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }
    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Search for text patterns in files".to_string()
    }

    fn detail(&self) -> String {
        "Recursively search for lines matching a pattern in files.\
         Takes `pattern` (string or regex), `path` (directory, default \".\"),\
         and optional `is_regex` (bool, default false).\
         Returns a JSON array of `{file, line_number, line}` objects.\
         Read-only: does not modify any file."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Search pattern or regex"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default .)"
                },
                "is_regex": {
                    "type": "boolean",
                    "description": "Treat pattern as regex (default false)"
                }
            },
            "required": ["pattern"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            is_expensive: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let pattern = required_str(&args, "pattern")?;
        let dir = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let is_regex = args.get("is_regex") == Some(&Value::Bool(true));
        let re = if is_regex {
            Regex::new(pattern)
                .map_err(|e| ToolCallError::InvalidArgs(format!("invalid regex: {e}")))?
        } else {
            Regex::new(&regex::escape(pattern))
                .map_err(|e| ToolCallError::InvalidArgs(format!("regex error: {e}")))?
        };
        let deps = (
            self.permission_engine.clone(),
            self.session_manager.clone(),
            self.config_manager.clone(),
            self.approval_flow.clone(),
        );
        check_and_execute(&deps, ctx, dir, "read", async move {
            let mut results = Vec::new();
            grep_walk(Path::new(dir), &re, &mut results);
            Ok(ToolResult {
                data: serde_json::json!({ "results": results }),
                new_messages: vec![],
                context_modifier: None,
            })
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// LsTool
// ---------------------------------------------------------------------------

pub struct LsTool {
    permission_engine: PermEngine,
    session_manager: SessionMgr,
    config_manager: ConfigMgr,
    approval_flow: ApprovalMtx,
}

impl LsTool {
    pub fn new(perm: PermEngine, sm: SessionMgr, cm: ConfigMgr, af: ApprovalMtx) -> Self {
        Self {
            permission_engine: perm,
            session_manager: sm,
            config_manager: cm,
            approval_flow: af,
        }
    }
}

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "Ls"
    }
    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "List directory entries".to_string()
    }

    fn detail(&self) -> String {
        "List entries in a directory.\
         Takes optional `path` (directory, default \".\").\
         Returns a JSON array of entry names.\
         Read-only: does not modify any file or directory."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to list (default .)"
                }
            },
            "required": []
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let deps = (
            self.permission_engine.clone(),
            self.session_manager.clone(),
            self.config_manager.clone(),
            self.approval_flow.clone(),
        );
        check_and_execute(&deps, ctx, path, "read", list_dir(path)).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "file_ops_tests.rs"]
mod tests;
