//! Unit tests for dynamic prompt generation across tools.
//!
//! These tests verify that the `generate_prompt` method on `BashTool`
//! adapts its output to the runtime context: it embeds the workdir
//! path when one is present, and the git branch / uncommitted changes
//! when the workdir carries git information. Without a workdir, the
//! implementation must fall back to the static `detail()` string.
//!
//! These tests live in a separate file (rather than inside
//! `src/tools/mod.rs`'s inline `mod tests`) so they can be run in
//! isolation via `cargo test --lib prompt_generation_tests`.

use std::sync::Arc;

use crate::builtin::BashTool;
use crate::WorkdirContext;
use crate::{PromptGenerationContext, Tool};
use closeclaw_gateway::GatewayConfig;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::RuleSet;
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use tempfile::TempDir;

// --- Mock TaskManager ---

struct MockTaskManager;

#[async_trait::async_trait]
impl closeclaw_tasks::TaskManager for MockTaskManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        unimplemented!("mock")
    }
    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        _command: &str,
    ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
        unimplemented!("mock")
    }
    async fn kill_task(&self, _task_id: &str) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
        unimplemented!("mock")
    }
    async fn get_task(&self, _task_id: &str) -> Option<closeclaw_tasks::BackgroundTask> {
        None
    }
}

// --- Helpers (mirror the helpers in src/tools/builtin/bash_tests.rs) ---

fn test_permission_engine() -> Arc<PermissionEngine> {
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

fn test_bg_manager() -> Arc<dyn closeclaw_tasks::TaskManager> {
    Arc::new(MockTaskManager)
}

fn test_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: closeclaw_gateway::DmScope::default(),
            ..Default::default()
        },
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

fn test_config_manager() -> Arc<closeclaw_config::ConfigManager> {
    let tmp = tempfile::TempDir::new().unwrap();
    Arc::new(
        closeclaw_config::ConfigManager::new(tmp.path().to_path_buf())
            .expect("ConfigManager::new should succeed"),
    )
}

fn test_bash_tool() -> BashTool {
    BashTool::new(
        test_permission_engine(),
        test_bg_manager(),
        test_session_manager(),
        test_config_manager(),
        correct_approval_flow(),
    )
}

fn correct_approval_flow() -> Arc<tokio::sync::Mutex<ApprovalFlow>> {
    Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
        Arc::clone(&test_session_manager()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )))
}

// --- generate_prompt: workdir-aware behavior ---

/// `generate_prompt` should embed the workdir path into the
/// returned prompt when one is present in the context.
#[test]
fn test_generate_prompt_with_workdir() {
    let tmp = TempDir::new().unwrap();
    let workdir_path = tmp
        .path()
        .join("example-workdir")
        .to_string_lossy()
        .to_string();
    let tool = test_bash_tool();
    let ctx = PromptGenerationContext {
        agent_id: "agent-1".to_string(),
        workdir: Some(WorkdirContext {
            path: workdir_path.clone(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        available_tool_names: vec!["Bash".to_string()],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
    };

    let prompt = tool.generate_prompt(&ctx);

    // Primary assertion: the workdir path is in the rendered prompt.
    assert!(
        prompt.contains(&workdir_path),
        "expected prompt to contain workdir path, got: {}",
        prompt
    );

    // Sanity check: the static `detail()` body is preserved as a prefix.
    assert_eq!(
        prompt,
        format!("{} Working directory: {}.", tool.detail(), workdir_path)
    );
}

/// `generate_prompt` should fall back to the static `detail()` body
/// when no workdir is set in the context.
#[test]
fn test_generate_prompt_without_workdir() {
    let tool = test_bash_tool();
    let ctx = PromptGenerationContext {
        agent_id: "agent-1".to_string(),
        workdir: None,
        available_tool_names: vec!["Bash".to_string()],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
    };

    let prompt = tool.generate_prompt(&ctx);

    // Without a workdir, generate_prompt must return exactly the same
    // string as `detail()`.
    assert_eq!(prompt, tool.detail());
}

/// `generate_prompt` should surface git information (branch name and
/// the number of uncommitted changes) when the workdir context is a
/// git repository.
#[test]
fn test_bash_prompt_includes_git_info() {
    let tmp = TempDir::new().unwrap();
    let workdir_path = tmp.path().join("git-repo").to_string_lossy().to_string();
    let tool = test_bash_tool();
    let ctx = PromptGenerationContext {
        agent_id: "agent-1".to_string(),
        workdir: Some(WorkdirContext {
            path: workdir_path.clone(),
            has_git: true,
            branch: Some("feat/dynamic-prompt-generation".to_string()),
            recent_changes: 3,
        }),
        available_tool_names: vec!["Bash".to_string()],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
    };

    let prompt = tool.generate_prompt(&ctx);

    // The branch name must be present in the rendered prompt.
    assert!(
        prompt.contains("feat/dynamic-prompt-generation"),
        "expected prompt to contain branch name, got: {}",
        prompt
    );

    // The uncommitted changes count must be reported.
    assert!(
        prompt.contains("3 uncommitted change(s)"),
        "expected prompt to mention 3 uncommitted changes, got: {}",
        prompt
    );

    // And the workdir path itself must still be present.
    assert!(
        prompt.contains(&workdir_path),
        "expected prompt to contain workdir path, got: {}",
        prompt
    );
}
