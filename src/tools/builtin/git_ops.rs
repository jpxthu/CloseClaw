//! Built-in tools — git operations (Tool trait implementation).
//!
//! Each tool wraps a git subcommand via `std::process::Command`,
//! independent from the [`crate::skills`] module.

use crate::tools::{Tool, ToolContext, ToolError, ToolFlags};

use serde_json::Value;

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// GitStatusTool
// ---------------------------------------------------------------------------

pub struct GitStatusTool;

impl Default for GitStatusTool {
    fn default() -> Self {
        Self
    }
}

impl GitStatusTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "GitStatus"
    }

    fn group(&self) -> &str {
        "git_ops"
    }

    fn summary(&self) -> String {
        "Get git working tree status".to_string()
    }

    fn detail(&self) -> String {
        "Run `git status --porcelain` to show the current working tree status.\
         Returns JSON with `output` field containing the porcelain-formatted status."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
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
}

// ---------------------------------------------------------------------------
// GitLogTool
// ---------------------------------------------------------------------------

pub struct GitLogTool;

impl Default for GitLogTool {
    fn default() -> Self {
        Self
    }
}

impl GitLogTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for GitLogTool {
    fn name(&self) -> &str {
        "GitLog"
    }

    fn group(&self) -> &str {
        "git_ops"
    }

    fn summary(&self) -> String {
        "Show recent git commits".to_string()
    }

    fn detail(&self) -> String {
        "Run `git log --oneline -10` to show the 10 most recent commits.\
         Returns JSON with `output` field containing the commit log."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
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
}

// ---------------------------------------------------------------------------
// GitCommitTool
// ---------------------------------------------------------------------------

pub struct GitCommitTool;

impl Default for GitCommitTool {
    fn default() -> Self {
        Self
    }
}

impl GitCommitTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for GitCommitTool {
    fn name(&self) -> &str {
        "GitCommit"
    }

    fn group(&self) -> &str {
        "git_ops"
    }

    fn summary(&self) -> String {
        "Commit staged changes with a message".to_string()
    }

    fn detail(&self) -> String {
        "Run `git commit -m <message>` to commit staged changes.\
         Takes `message` (required), returns JSON with `success`, `output`, `error`."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Commit message"
                }
            },
            "required": ["message"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_destructive: true,
            is_expensive: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// GitPushTool
// ---------------------------------------------------------------------------

pub struct GitPushTool;

impl Default for GitPushTool {
    fn default() -> Self {
        Self
    }
}

impl GitPushTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for GitPushTool {
    fn name(&self) -> &str {
        "GitPush"
    }

    fn group(&self) -> &str {
        "git_ops"
    }

    fn summary(&self) -> String {
        "Push commits to remote".to_string()
    }

    fn detail(&self) -> String {
        "Run `git push` to push commits to the remote repository.\
         Returns JSON with `success`, `output`, `error`."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_destructive: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// GitPullTool
// ---------------------------------------------------------------------------

pub struct GitPullTool;

impl Default for GitPullTool {
    fn default() -> Self {
        Self
    }
}

impl GitPullTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for GitPullTool {
    fn name(&self) -> &str {
        "GitPull"
    }

    fn group(&self) -> &str {
        "git_ops"
    }

    fn summary(&self) -> String {
        "Pull commits from remote".to_string()
    }

    fn detail(&self) -> String {
        "Run `git pull` to pull commits from the remote repository.\
         Returns JSON with `success`, `output`, `error`."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_destructive: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        }
    }

    // --- GitStatusTool ----------------------------------------------------

    #[test]
    fn test_git_status_name() {
        let tool = GitStatusTool::new();
        assert_eq!(tool.name(), "GitStatus");
    }

    #[test]
    fn test_git_status_group() {
        let tool = GitStatusTool::new();
        assert_eq!(tool.group(), "git_ops");
    }

    #[test]
    fn test_git_status_summary_len() {
        let tool = GitStatusTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_git_status_flags_read_only() {
        let tool = GitStatusTool::new();
        assert!(tool.flags().is_read_only);
        assert!(!tool.flags().is_destructive);
    }

    #[test]
    fn test_git_status_schema_no_required() {
        let tool = GitStatusTool::new();
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }

    // --- GitLogTool -------------------------------------------------------

    #[test]
    fn test_git_log_name() {
        let tool = GitLogTool::new();
        assert_eq!(tool.name(), "GitLog");
    }

    #[test]
    fn test_git_log_group() {
        let tool = GitLogTool::new();
        assert_eq!(tool.group(), "git_ops");
    }

    #[test]
    fn test_git_log_summary_len() {
        let tool = GitLogTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_git_log_flags_read_only() {
        let tool = GitLogTool::new();
        assert!(tool.flags().is_read_only);
        assert!(!tool.flags().is_destructive);
    }

    #[test]
    fn test_git_log_schema_no_required() {
        let tool = GitLogTool::new();
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }

    // --- GitCommitTool ----------------------------------------------------

    #[test]
    fn test_git_commit_name() {
        let tool = GitCommitTool::new();
        assert_eq!(tool.name(), "GitCommit");
    }

    #[test]
    fn test_git_commit_group() {
        let tool = GitCommitTool::new();
        assert_eq!(tool.group(), "git_ops");
    }

    #[test]
    fn test_git_commit_summary_len() {
        let tool = GitCommitTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_git_commit_flags_destructive_expensive() {
        let tool = GitCommitTool::new();
        assert!(tool.flags().is_destructive);
        assert!(tool.flags().is_expensive);
        assert!(!tool.flags().is_read_only);
    }

    #[test]
    fn test_git_commit_schema_has_message() {
        let tool = GitCommitTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("message"));
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("message")));
    }

    // --- GitPushTool ------------------------------------------------------

    #[test]
    fn test_git_push_name() {
        let tool = GitPushTool::new();
        assert_eq!(tool.name(), "GitPush");
    }

    #[test]
    fn test_git_push_group() {
        let tool = GitPushTool::new();
        assert_eq!(tool.group(), "git_ops");
    }

    #[test]
    fn test_git_push_summary_len() {
        let tool = GitPushTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_git_push_flags_destructive() {
        let tool = GitPushTool::new();
        assert!(tool.flags().is_destructive);
        assert!(!tool.flags().is_read_only);
    }

    #[test]
    fn test_git_push_schema_no_required() {
        let tool = GitPushTool::new();
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }

    // --- GitPullTool ------------------------------------------------------

    #[test]
    fn test_git_pull_name() {
        let tool = GitPullTool::new();
        assert_eq!(tool.name(), "GitPull");
    }

    #[test]
    fn test_git_pull_group() {
        let tool = GitPullTool::new();
        assert_eq!(tool.group(), "git_ops");
    }

    #[test]
    fn test_git_pull_summary_len() {
        let tool = GitPullTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_git_pull_flags_destructive() {
        let tool = GitPullTool::new();
        assert!(tool.flags().is_destructive);
        assert!(!tool.flags().is_read_only);
    }

    #[test]
    fn test_git_pull_schema_no_required() {
        let tool = GitPullTool::new();
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }
}
