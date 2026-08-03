//! File operations skill
use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillError, SkillListingMeta, SkillManifest};
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;

use tokio::task::spawn_blocking;

pub struct FileOpsSkill;

impl Default for FileOpsSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl FileOpsSkill {
    pub fn new() -> Self {
        Self
    }

    /// Build the default capability description returned when no
    /// action is specified.
    fn capabilities_description() -> String {
        json!({
            "skill": "file_ops",
            "description": "File system operations: read, write, list, delete",
            "supported_actions": ["read", "list", "stat"],
            "usage": {
                "read": {"path": "<file_path>"},
                "list": {"path": "<directory_path>"},
                "stat": {"path": "<file_path>"}
            }
        })
        .to_string()
    }

    /// Read file contents asynchronously via spawn_blocking.
    async fn execute_read(path: &str) -> Result<String, SkillError> {
        let path = path.to_string();
        spawn_blocking(move || {
            let p = Path::new(&path);
            if !p.exists() {
                return Err(SkillError::ExecutionFailed(format!(
                    "file not found: {path}"
                )));
            }
            if p.is_dir() {
                return Err(SkillError::InvalidArgs(format!(
                    "path is a directory, not a file: {path}"
                )));
            }
            match std::fs::read_to_string(p) {
                Ok(content) => Ok(json!({
                    "action": "read",
                    "path": path,
                    "content": content,
                    "size_bytes": content.len()
                })
                .to_string()),
                Err(e) => Err(SkillError::ExecutionFailed(format!(
                    "failed to read {path}: {e}"
                ))),
            }
        })
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("task join error: {e}")))?
    }

    /// List directory entries asynchronously via spawn_blocking.
    async fn execute_list(path: &str) -> Result<String, SkillError> {
        let path = path.to_string();
        spawn_blocking(move || {
            let p = Path::new(&path);
            if !p.exists() {
                return Err(SkillError::ExecutionFailed(format!(
                    "directory not found: {path}"
                )));
            }
            if !p.is_dir() {
                return Err(SkillError::InvalidArgs(format!(
                    "path is not a directory: {path}"
                )));
            }
            match std::fs::read_dir(p) {
                Ok(entries) => {
                    let items: Vec<serde_json::Value> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| {
                            let file_type = e.file_type().ok();
                            json!({
                                "name": e.file_name().to_string_lossy().to_string(),
                                "is_dir": file_type.is_some_and(|t| t.is_dir()),
                                "is_file": file_type.is_some_and(|t| t.is_file()),
                            })
                        })
                        .collect();
                    Ok(json!({
                        "action": "list",
                        "path": path,
                        "count": items.len(),
                        "entries": items
                    })
                    .to_string())
                }
                Err(e) => Err(SkillError::ExecutionFailed(format!(
                    "failed to list {path}: {e}"
                ))),
            }
        })
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("task join error: {e}")))?
    }

    /// Return file/directory stat information asynchronously via
    /// spawn_blocking.
    async fn execute_stat(path: &str) -> Result<String, SkillError> {
        let path = path.to_string();
        spawn_blocking(move || {
            let p = Path::new(&path);
            if !p.exists() {
                return Err(SkillError::ExecutionFailed(format!(
                    "path not found: {path}"
                )));
            }
            match std::fs::metadata(p) {
                Ok(meta) => Ok(json!({
                    "action": "stat",
                    "path": path,
                    "is_file": meta.is_file(),
                    "is_dir": meta.is_dir(),
                    "size_bytes": meta.len(),
                    "readonly": meta.permissions().readonly(),
                })
                .to_string()),
                Err(e) => Err(SkillError::ExecutionFailed(format!(
                    "failed to stat {path}: {e}"
                ))),
            }
        })
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("task join error: {e}")))?
    }
}

#[async_trait]
impl Skill for FileOpsSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "file_ops".to_string(),
            version: "1.0.0".to_string(),
            description: "File system operations: read, write, list, delete".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# File Operations Skill

You have access to file system tools. Use them to perform file operations:

- **Read a file**: Use the `read` tool with the file path.
- **Write a file**: Use the `write` tool with the file path and content.
- **Check if a file exists**: Use the `read` tool and check for errors,
  or use `exec` with `test -f <path>`.
- **List directory contents**: Use `exec` with `ls <path>` or `find <path> -maxdepth 1`.
- **Delete a file**: Use `exec` with `rm <path>`.

Always confirm destructive operations with the user before executing."#
    }

    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let args = match args {
            Some(a) => a,
            None => return Ok(Self::capabilities_description()),
        };

        let action = args.get("action").and_then(|v| v.as_str());
        let path = args.get("path").and_then(|v| v.as_str());

        match action {
            None => Ok(Self::capabilities_description()),
            Some("read") => {
                let p = path.ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'path' for read action".into())
                })?;
                Self::execute_read(p).await
            }
            Some("list") => {
                let p = path.ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'path' for list action".into())
                })?;
                Self::execute_list(p).await
            }
            Some("stat") => {
                let p = path.ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'path' for stat action".into())
                })?;
                Self::execute_stat(p).await
            }
            Some(other) => Err(SkillError::InvalidArgs(format!(
                "unknown action '{other}', supported: read, list, stat"
            ))),
        }
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to read, \
                write, list, or delete files on disk"
                .to_string(),
            user_invocable: false,
            paths: vec![],
            effort: SkillEffort::Small,
        }
    }
}
