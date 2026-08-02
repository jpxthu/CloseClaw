//! Git operations skill
use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillError, SkillListingMeta, SkillManifest};
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;

use tokio::task::spawn_blocking;

#[derive(Default)]
pub struct GitOpsSkill;

impl GitOpsSkill {
    pub fn new() -> Self {
        Self
    }

    /// Build the default capability description returned when no
    /// action is specified.
    fn capabilities_description() -> String {
        json!({
            "skill": "git_ops",
            "description": "Git operations: status, commit, push, pull",
            "supported_actions": ["status", "log", "diff"],
            "usage": {
                "status": {"path": "<repo_path>"},
                "log": {"path": "<repo_path>", "max_count": 10},
                "diff": {"path": "<repo_path>", "staged": false}
            }
        })
        .to_string()
    }

    /// Resolve the working directory for git commands. Defaults to
    /// current directory when not provided.
    fn resolve_workdir(path: Option<&str>) -> Result<std::path::PathBuf, SkillError> {
        match path {
            Some(p) => Ok(std::path::PathBuf::from(p)),
            None => std::env::current_dir().map_err(|e| {
                SkillError::ExecutionFailed(format!("failed to get current directory: {e}"))
            }),
        }
    }

    /// Run a git command via spawn_blocking and return stdout.
    async fn run_git(args: &[&str], workdir: &Path) -> Result<String, SkillError> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let wd = workdir.to_path_buf();
        spawn_blocking(move || {
            let output = std::process::Command::new("git")
                .args(&args)
                .current_dir(&wd)
                .output()
                .map_err(|e| SkillError::ExecutionFailed(format!("failed to run git: {e}")))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(SkillError::ExecutionFailed(format!(
                    "git {} failed: {}",
                    args.join(" "),
                    stderr.trim()
                )));
            }
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        })
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("task join error: {e}")))?
    }

    /// Execute `git status --porcelain`.
    async fn execute_status(workdir: &Path) -> Result<String, SkillError> {
        let stdout = Self::run_git(&["status", "--porcelain"], workdir).await?;
        Ok(json!({
            "action": "status",
            "output": stdout.trim(),
            "has_changes": !stdout.trim().is_empty()
        })
        .to_string())
    }

    /// Execute `git log --oneline -<max_count>`.
    async fn execute_log(workdir: &Path, max_count: Option<u32>) -> Result<String, SkillError> {
        let count = max_count.unwrap_or(10);
        let flag = format!("-{count}");
        let stdout = Self::run_git(&["log", "--oneline", &flag], workdir).await?;
        Ok(json!({
            "action": "log",
            "output": stdout.trim(),
            "count": stdout.lines().count()
        })
        .to_string())
    }

    /// Execute `git diff` (optionally `--staged`).
    async fn execute_diff(workdir: &Path, staged: bool) -> Result<String, SkillError> {
        let mut args = vec!["diff"];
        if staged {
            args.push("--staged");
        }
        let stdout = Self::run_git(&args, workdir).await?;
        Ok(json!({
            "action": "diff",
            "output": stdout.trim(),
            "staged": staged,
            "has_diff": !stdout.trim().is_empty()
        })
        .to_string())
    }
}

#[async_trait]
impl Skill for GitOpsSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "git_ops".to_string(),
            version: "1.0.0".to_string(),
            description: "Git operations: status, commit, push, pull".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn body(&self) -> &str {
        r#"# Git Operations Skill

You have access to the `exec` tool. Use it to run git commands:

- **Status**: `exec` with `git status --porcelain`
- **Log**: `exec` with `git log --oneline -10`
- **Commit**: `exec` with `git commit -m "<message>"`
- **Push**: `exec` with `git push`
- **Pull**: `exec` with `git pull`
- **Diff**: `exec` with `git diff`

Always ensure changes are staged before committing. Confirm destructive operations (force push, reset) with the user."#
    }

    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let args = match args {
            Some(a) => a,
            None => return Ok(Self::capabilities_description()),
        };

        let action = args.get("action").and_then(|v| v.as_str());
        let path = args.get("path").and_then(|v| v.as_str());
        let workdir = Self::resolve_workdir(path)?;

        match action {
            None => Ok(Self::capabilities_description()),
            Some("status") => Self::execute_status(&workdir).await,
            Some("log") => {
                let max_count = args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32);
                Self::execute_log(&workdir, max_count).await
            }
            Some("diff") => {
                let staged = args
                    .get("staged")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Self::execute_diff(&workdir, staged).await
            }
            Some(other) => Err(SkillError::InvalidArgs(format!(
                "unknown action '{other}', supported: status, log, diff"
            ))),
        }
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to perform git \
                operations like commit, push, pull, or diff"
                .to_string(),
            user_invocable: false,
            paths: vec![],
            effort: SkillEffort::Small,
        }
    }
}
