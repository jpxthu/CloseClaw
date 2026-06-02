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

use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::rules::RuleSetBuilder;
use crate::system_prompt::WorkdirContext;
use crate::tasks::BackgroundTaskManager;
use crate::tools::builtin::BashTool;
use crate::tools::{PromptGenerationContext, Tool};

// --- Helpers (mirror the helpers in src/tools/builtin/bash_tests.rs) ---

fn test_permission_engine() -> Arc<PermissionEngine> {
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

fn test_bg_manager() -> Arc<BackgroundTaskManager> {
    Arc::new(BackgroundTaskManager::new())
}

fn test_bash_tool() -> BashTool {
    BashTool::new(test_permission_engine(), test_bg_manager())
}

// --- generate_prompt: workdir-aware behavior ---

/// `generate_prompt` should embed the workdir path into the
/// returned prompt when one is present in the context.
#[test]
fn test_generate_prompt_with_workdir() {
    let tool = test_bash_tool();
    let ctx = PromptGenerationContext {
        agent_id: "agent-1".to_string(),
        workdir: Some(WorkdirContext {
            path: "/tmp/example-workdir".to_string(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        available_tool_names: vec!["Bash".to_string()],
    };

    let prompt = tool.generate_prompt(&ctx);

    // Primary assertion: the workdir path is in the rendered prompt.
    assert!(
        prompt.contains("/tmp/example-workdir"),
        "expected prompt to contain workdir path, got: {}",
        prompt
    );

    // Sanity check: the static `detail()` body is preserved as a prefix.
    assert_eq!(
        prompt,
        format!("{} Working directory: /tmp/example-workdir.", tool.detail())
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
    let tool = test_bash_tool();
    let ctx = PromptGenerationContext {
        agent_id: "agent-1".to_string(),
        workdir: Some(WorkdirContext {
            path: "/tmp/git-repo".to_string(),
            has_git: true,
            branch: Some("feat/dynamic-prompt-generation".to_string()),
            recent_changes: 3,
        }),
        available_tool_names: vec!["Bash".to_string()],
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
        prompt.contains("/tmp/git-repo"),
        "expected prompt to contain workdir path, got: {}",
        prompt
    );
}
