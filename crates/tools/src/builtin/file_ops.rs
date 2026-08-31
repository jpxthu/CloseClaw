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
use crate::{PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use closeclaw_common::ReadRange;

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
    if let Some(r) =
        permission_check::check_tool_permission(deps, ctx, "file_ops", "call", None).await?
    {
        return Ok(r);
    }
    if let Some(r) = permission_check::check_file_op_permission(deps, ctx, path, op, None).await? {
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

/// "When to use" section for the Read tool prompt.
fn read_prompt_when_to_use() -> String {
    "Use Read to view file contents, confirm file existence, read configurations, \
     or inspect code. Accepts a file path and returns text content. \
     Supports large files via offset/limit parameters: specify offset \
     (1-indexed line number) and limit (max lines) to read in chunks. \
     Large files are automatically truncated with a continuation hint \
     showing the next offset to use. For images, use the image \
     analysis tool instead."
        .to_string()
}

/// Append workdir-based path guidance to the prompt parts.
fn read_prompt_add_workdir_guidance(context: &PromptGenerationContext, parts: &mut Vec<String>) {
    if let Some(ref wd) = context.workdir {
        parts.push(closeclaw_common::format_workdir_guidance(
            wd,
            "Relative paths are resolved against this directory. \
             Use absolute paths for files outside the working directory.",
        ));
    }
}

/// "Usage principles" section for the Read tool prompt.
fn read_prompt_usage_principles() -> String {
    "For large files, use offset and limit parameters to read in chunks. \
     Files exceeding 50KB or 2000 lines are truncated automatically — \
     the response includes a continuation hint with the exact offset \
     for the next read. Do not attempt to read entire binary files \
     or very large files in a single call — read only the relevant portions. \
     If the file is an image, use the image analysis tool instead of Read."
        .to_string()
}

/// Append combination suggestions based on available tools.
fn read_prompt_add_combination_suggestions(
    context: &PromptGenerationContext,
    parts: &mut Vec<String>,
) {
    let has_write = context
        .available_tool_names
        .iter()
        .any(|t| t == "Write" || t == "write");
    let has_edit = context
        .available_tool_names
        .iter()
        .any(|t| t == "Edit" || t == "edit");
    let has_exec = context
        .available_tool_names
        .iter()
        .any(|t| t == "Bash" || t == "bash" || t == "exec");
    let mut suggestions = Vec::new();
    if has_write || has_edit {
        suggestions.push("Write/Edit (read file, then modify it)");
    }
    if has_exec {
        suggestions.push("Bash (read log files, then analyze output)");
    }
    if !suggestions.is_empty() {
        parts.push(format!("Combine with: {}.", suggestions.join(", ")));
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
        "Read the contents of a file given its path.\
         Returns the text content as a JSON object with key `content`.\
         Supports offset/limit parameters for pagination of large files.\
         Large files are automatically truncated with a continuation hint.\
         Fails if the path does not exist or is not a readable file."
            .to_string()
    }

    fn generate_prompt(&self, context: &PromptGenerationContext) -> String {
        let mut parts = Vec::new();
        parts.push(read_prompt_when_to_use());
        read_prompt_add_workdir_guidance(context, &mut parts);
        parts.push(read_prompt_usage_principles());
        read_prompt_add_combination_suggestions(context, &mut parts);
        parts.join("\n\n")
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workdir-relative file path"
                },
                "offset": {
                    "type": "number",
                    "description": "1-indexed line number to start reading from"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of lines to read"
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
        let offset = args
            .get("offset")
            .and_then(Value::as_f64)
            .map(|v| v as usize)
            .unwrap_or(1);
        let limit = args
            .get("limit")
            .and_then(Value::as_f64)
            .map(|v| v as usize);
        let deps = (
            self.permission_engine.clone(),
            self.session_manager.clone(),
            self.config_manager.clone(),
            self.approval_flow.clone(),
        );
        check_and_execute(&deps, ctx, path, "read", async {
            let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
            if let Some(cached) = check_dedup_cache(ctx, path, mtime, offset, limit) {
                return Ok(cached);
            }
            read_and_truncate(path, offset, limit, mtime, ctx).await
        })
        .await
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
        let path_owned = path.to_string();
        check_and_execute(&deps, ctx, path, "write", async move {
            // Staleness check: ensure file hasn't changed since last Read.
            if Path::new(&path_owned).exists() {
                check_staleness(ctx, &path_owned).await?;
            }
            write_file(&path_owned, content).await
        })
        .await
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

/// Parse the `edits` JSON array into a vec of [`EditOp`]s.
///
/// Validates that each element has a non-empty `oldText` distinct from
/// `newText`.
fn parse_edits_array(
    arr: &[Value],
) -> Result<Vec<crate::builtin::edit_match::EditOp>, ToolCallError> {
    if arr.is_empty() {
        return Err(ToolCallError::InvalidArgs(
            "edits array must not be empty".to_string(),
        ));
    }
    let mut edits = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let old_text = item
            .get("oldText")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ToolCallError::InvalidArgs(format!(
                    "edits[{i}]: oldText is required and must not be empty"
                ))
            })?;
        let new_text = item.get("newText").and_then(Value::as_str).unwrap_or("");
        if old_text == new_text {
            return Err(ToolCallError::InvalidArgs(format!(
                "edits[{i}]: oldText and newText must be different"
            )));
        }
        edits.push(crate::builtin::edit_match::EditOp {
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
        });
    }
    Ok(edits)
}

/// Parse the legacy single-edit format (`oldText`/`newText` at top level).
fn parse_legacy_edit(args: &Value) -> Result<crate::builtin::edit_match::EditOp, ToolCallError> {
    let old_text = args
        .get("oldText")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolCallError::InvalidArgs("missing required parameter: oldText or edits".to_string())
        })?;
    let new_text = args.get("newText").and_then(Value::as_str).unwrap_or("");
    if old_text == new_text {
        return Err(ToolCallError::InvalidArgs(
            "oldText and newText must be different".to_string(),
        ));
    }
    Ok(crate::builtin::edit_match::EditOp {
        old_text: old_text.to_string(),
        new_text: new_text.to_string(),
    })
}

/// Parse the `edits` array from args, falling back to the legacy
/// `oldText`/`newText` single-edit format.
///
/// Returns `(edits_vec, replace_all)`.
fn parse_edits(
    args: &Value,
) -> Result<(Vec<crate::builtin::edit_match::EditOp>, bool), ToolCallError> {
    let replace_all = args.get("replace_all") == Some(&Value::Bool(true));

    if let Some(arr) = args.get("edits").and_then(Value::as_array) {
        let edits = parse_edits_array(arr)?;
        return Ok((edits, replace_all));
    }

    let edit = parse_legacy_edit(args)?;
    Ok((vec![edit], replace_all))
}

/// Staleness check: verify file mtime matches what was recorded during
/// the last Read. Returns Ok(()) if the file was never read (backward
/// compatible) or if mtime is consistent; Err otherwise.
async fn check_staleness(ctx: &ToolContext, path: &str) -> Result<(), ToolCallError> {
    let session = match ctx.session.as_ref() {
        Some(s) => s,
        None => return Ok(()), // test scenario — no session
    };
    let Some(recorded) = session.get_file_mtime(path) else {
        // File was never read — allow (backward compatible).
        return Ok(());
    };
    let current_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    match current_mtime {
        Some(current) if current == recorded => Ok(()),
        Some(_) => Err(ToolCallError::ExecutionFailed(
            "file has been modified since last read; re-read the file before editing".to_string(),
        )),
        None => Ok(()), // can't determine mtime — allow
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
        "Apply targeted edits to a file".to_string()
    }

    fn detail(&self) -> String {
        "Apply targeted edits to an existing file using exact text replacement.\
         Accepts an `edits` array where each element has `oldText` and `newText`.\
         Supports multiple replacements in a single call with non-incremental matching.\
         Falls back to legacy `oldText`/`newText` single-edit format.\
         Fails if any `oldText` is not found in the file.\
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
                "edits": {
                    "type": "array",
                    "description": "Array of edit operations, each with oldText and newText",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": {
                                "type": "string",
                                "description": "Exact text to search for and replace"
                            },
                            "newText": {
                                "type": "string",
                                "description": "Replacement text"
                            }
                        },
                        "required": ["oldText", "newText"]
                    }
                },
                "oldText": {
                    "type": "string",
                    "description": "Legacy: exact text to search for (use edits array instead)"
                },
                "newText": {
                    "type": "string",
                    "description": "Legacy: replacement text (use edits array instead)"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences instead of requiring exactly one match"
                }
            },
            "required": ["path"]
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
        let (edits, replace_all) = parse_edits(&args)?;
        let deps = (
            self.permission_engine.clone(),
            self.session_manager.clone(),
            self.config_manager.clone(),
            self.approval_flow.clone(),
        );
        let path_owned = path.to_string();
        check_and_execute(&deps, ctx, path, "write", async move {
            // Staleness check: ensure file hasn't changed since last Read.
            check_staleness(ctx, &path_owned).await?;

            let content = std::fs::read_to_string(&path_owned)
                .map_err(|e| ToolCallError::ExecutionFailed(format!("{path_owned}: {e}")))?;
            let updated =
                crate::builtin::edit_match::match_and_apply(&content, &edits, replace_all)
                    .map_err(|e| ToolCallError::ExecutionFailed(e.to_string()))?;
            std::fs::write(&path_owned, &updated)
                .map_err(|e| ToolCallError::ExecutionFailed(format!("{path_owned}: {e}")))?;
            Ok(ToolResult {
                data: serde_json::json!({ "content": updated }),
                new_messages: vec![],
                context_modifier: None,
            })
        })
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
// ReadTool helpers
// ---------------------------------------------------------------------------

/// Check dedup cache: same path + range + unchanged mtime → cached hint.
fn check_dedup_cache(
    ctx: &ToolContext,
    path: &str,
    mtime: Option<std::time::SystemTime>,
    offset: usize,
    limit: Option<usize>,
) -> Option<ToolResult> {
    let session = ctx.session.as_ref()?;
    let cache = session.get_file_read_cache(path)?;
    let range = ReadRange { offset, limit };
    if cache.mtime == mtime && cache.ranges.contains(&range) {
        Some(ToolResult {
            data: serde_json::json!({ "content": "File unchanged since last read." }),
            new_messages: vec![],
            context_modifier: None,
        })
    } else {
        None
    }
}

/// Read file, apply truncation, and record range for dedup.
async fn read_and_truncate(
    path: &str,
    offset: usize,
    limit: Option<usize>,
    mtime: Option<std::time::SystemTime>,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolCallError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ToolCallError::ExecutionFailed(format!("{path}: {e}")))?;
    let config = super::read_truncator::TruncationConfig::default();
    let result = super::read_truncator::truncate_lines(&raw, offset, limit, &config);
    let truncation_msg = super::read_truncator::format_truncation_message(&result, offset);
    let mut output = result.content;
    if let Some(msg) = truncation_msg {
        output.push_str(&msg);
    }
    if let Some(session) = ctx.session.as_ref() {
        session.record_file_read(path, mtime).await;
        session
            .record_file_read_range(path, mtime, ReadRange { offset, limit })
            .await;
    }
    Ok(ToolResult {
        data: serde_json::json!({ "content": output }),
        new_messages: vec![],
        context_modifier: None,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "file_ops_tests.rs"]
pub(crate) mod tests;
