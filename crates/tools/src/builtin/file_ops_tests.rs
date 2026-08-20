//! Unit tests for file_ops tools — metadata and permission-check tests.

use super::*;
use closeclaw_common::{PromptGenerationContext, WorkdirContext};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{Action, Effect, Rule, RuleSet};
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_permission::Defaults;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

pub(crate) fn make_engine(rules: Vec<Rule>) -> PermEngine {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Deny,
            file_read: Effect::Deny,
            file_write: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rs),
    ))
}

pub(crate) fn make_sm() -> SessionMgr {
    use closeclaw_gateway::GatewayConfig;
    use closeclaw_session::persistence::ReasoningLevel;
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            ..Default::default()
        },
        None,
        None,
        ReasoningLevel::default(),
    ))
}

pub(crate) fn make_cm() -> ConfigMgr {
    let tmp = TempDir::new().unwrap();
    Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    )
}

pub(crate) fn make_af() -> ApprovalMtx {
    Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )))
}

/// Denying approval flow — submit_denial returns None (hard deny path).
fn make_af_deny() -> ApprovalMtx {
    Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new_deny_all(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )))
}

pub(crate) fn make_ctx(agent: &str) -> ToolContext {
    ToolContext {
        agent_id: agent.to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
    }
}

pub(crate) fn allow_tool(agent: &str, skill: &str) -> Rule {
    Rule {
        name: format!("allow-{skill}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::ToolCall {
            skill: skill.to_string(),
            methods: vec!["call".to_string()],
        }],
        template: None,
        priority: 0,
    }
}

pub(crate) fn allow_file(agent: &str, path_glob: &str, op: &str) -> Rule {
    Rule {
        name: format!("allow-file-{op}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::File {
            operation: op.to_string(),
            paths: vec![path_glob.to_string()],
        }],
        template: None,
        priority: 0,
    }
}

fn allow_config_write_rule(agent: &str) -> Rule {
    Rule {
        name: format!("allow-cfgwrite-{agent}"),
        subject: Rule::parse_subject(agent),
        effect: Effect::Allow,
        actions: vec![Action::ConfigWrite {
            files: vec!["*".to_string()],
        }],
        template: None,
        priority: 0,
    }
}

// ---------------------------------------------------------------------------
// Metadata tests (migrated from inline)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_name_group_summary() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    assert_eq!(tool.name(), "Read");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_read_only);
    assert!(!tool.flags().is_destructive);
}

#[tokio::test]
async fn test_write_name_group_summary() {
    let tool = WriteTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    assert_eq!(tool.name(), "Write");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_destructive);
    assert!(!tool.flags().is_read_only);
}

#[tokio::test]
async fn test_edit_name_group_summary() {
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    assert_eq!(tool.name(), "Edit");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_destructive);
    assert!(!tool.flags().is_read_only);
}

#[tokio::test]
async fn test_grep_name_group_summary() {
    let tool = GrepTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    assert_eq!(tool.name(), "Grep");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_read_only);
}

#[tokio::test]
async fn test_ls_name_group_summary() {
    let tool = LsTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    assert_eq!(tool.name(), "Ls");
    assert_eq!(tool.group(), "file_ops");
    assert!(tool.summary().len() <= 50);
    assert!(tool.flags().is_read_only);
}

#[tokio::test]
async fn test_read_input_schema_has_path() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("path"));
}

#[tokio::test]
async fn test_write_input_schema_has_path_and_content() {
    let tool = WriteTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("path"));
    assert!(props.contains_key("content"));
}

#[tokio::test]
async fn test_edit_input_schema_has_all_fields() {
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("path"));
    assert!(props.contains_key("edits"));
    assert!(props.contains_key("oldText"));
    assert!(props.contains_key("newText"));
    assert!(props.contains_key("replace_all"));
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.contains(&serde_json::json!("path")));
}

#[tokio::test]
async fn test_grep_input_schema_has_pattern() {
    let tool = GrepTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("pattern"));
}

#[tokio::test]
async fn test_ls_input_schema_optional_path() {
    let tool = LsTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let schema = tool.input_schema();
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.is_empty());
}

// ---------------------------------------------------------------------------
// Permission tests — ReadTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("test.txt");
    std::fs::write(&file, "hello").unwrap();
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({ "path": file.to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().data["content"], "hello\n");
}

#[tokio::test]
async fn test_read_denied_without_permission() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("secret.txt");
    std::fs::write(&file, "secret").unwrap();
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af_deny());
    let args = serde_json::json!({ "path": file.to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_read_denied_on_level1_only() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("data.txt");
    std::fs::write(&file, "data").unwrap();
    // Has FileOp rule but NO ToolCall rule
    let rules = vec![allow_file("a", "/tmp/**", "read")];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af_deny());
    let args = serde_json::json!({ "path": file.to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Permission tests — WriteTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("out.txt");
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = WriteTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "written"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "written");
}

#[tokio::test]
async fn test_write_denied_without_permission() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("blocked.txt");
    let tool = WriteTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af_deny());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "nope"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
    assert!(!path.exists());
}

// ---------------------------------------------------------------------------
// Permission tests — EditTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_edit_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("edit.txt");
    std::fs::write(&path, "old text here").unwrap();
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = EditTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "oldText": "old text",
        "newText": "new text"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "new text here");
}

#[tokio::test]
async fn test_edit_denied_without_permission() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("edit.txt");
    std::fs::write(&path, "original").unwrap();
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af_deny());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "oldText": "original",
        "newText": "changed"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "original");
}

// ---------------------------------------------------------------------------
// Permission tests — GrepTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_grep_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "target line").unwrap();
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = GrepTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "pattern": "target",
        "path": tmp.path().to_str().unwrap()
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let results = result.unwrap().data["results"].as_array().unwrap().clone();
    assert!(!results.is_empty());
}

#[tokio::test]
async fn test_grep_denied_without_permission() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "secret data").unwrap();
    let tool = GrepTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af_deny());
    let args = serde_json::json!({
        "pattern": "secret",
        "path": tmp.path().to_str().unwrap()
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Permission tests — LsTool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_ls_allowed_with_rules() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("file.txt"), "").unwrap();
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = LsTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({ "path": tmp.path().to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let tool_result = result.unwrap();
    let entries = tool_result.data["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e == "file.txt"));
}

#[tokio::test]
async fn test_ls_denied_without_permission() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("file.txt"), "").unwrap();
    let tool = LsTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af_deny());
    let args = serde_json::json!({ "path": tmp.path().to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// generate_prompt tests — ReadTool
// ---------------------------------------------------------------------------

/// `generate_prompt` must return context-aware output for empty context.
#[tokio::test]
async fn test_read_generate_prompt_empty_context() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let ctx = PromptGenerationContext::default();
    let prompt = tool.generate_prompt(&ctx);
    // Empty context: no workdir, no combination suggestions
    assert!(!prompt.is_empty(), "prompt must not be empty");
    assert!(
        prompt.contains("Read"),
        "prompt should mention the Read tool name"
    );
    assert!(
        !prompt.contains("Working directory"),
        "empty context should not mention working directory"
    );
}

/// Workdir context changes the prompt output.
#[tokio::test]
async fn test_read_generate_prompt_includes_workdir() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let no_workdir = PromptGenerationContext::default();
    let with_workdir = PromptGenerationContext {
        agent_id: "test-agent".into(),
        workdir: Some(WorkdirContext {
            path: "/some/path".into(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        ..Default::default()
    };
    let prompt_no = tool.generate_prompt(&no_workdir);
    let prompt_yes = tool.generate_prompt(&with_workdir);
    assert_ne!(
        prompt_no, prompt_yes,
        "different workdir contexts should produce different prompts"
    );
    assert!(
        prompt_yes.contains("/some/path"),
        "prompt should contain the working directory path"
    );
    assert!(
        prompt_yes.contains("not a git repo"),
        "non-git path should note absence of git"
    );
    assert!(
        prompt_yes.contains("Relative paths"),
        "workdir guidance should mention relative path resolution"
    );
}

/// Git branch and recent_changes are reflected in the prompt.
#[tokio::test]
async fn test_read_generate_prompt_includes_git_info() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let no_git = PromptGenerationContext {
        agent_id: "test-agent".into(),
        workdir: Some(WorkdirContext {
            path: "/tmp".into(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        ..Default::default()
    };
    let with_git = PromptGenerationContext {
        agent_id: "test-agent".into(),
        workdir: Some(WorkdirContext {
            path: "/tmp".into(),
            has_git: true,
            branch: Some("main".into()),
            recent_changes: 3,
        }),
        ..Default::default()
    };
    let prompt_no = tool.generate_prompt(&no_git);
    let prompt_yes = tool.generate_prompt(&with_git);
    assert!(
        prompt_yes.contains("main"),
        "git prompt should contain the branch name"
    );
    assert!(
        prompt_yes.contains("uncommitted change"),
        "git prompt should mention uncommitted changes"
    );
    assert!(
        prompt_no.contains("not a git repo"),
        "non-git prompt should note absence of git"
    );
}

/// Prompt includes combination suggestions when Write and Bash are available.
#[tokio::test]
async fn test_read_generate_prompt_combination_suggestions() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let ctx = PromptGenerationContext {
        available_tool_names: vec!["Read".into(), "Write".into(), "Bash".into()],
        ..Default::default()
    };
    let prompt = tool.generate_prompt(&ctx);
    assert!(
        prompt.contains("Write/Edit"),
        "should suggest Write/Edit as a combination"
    );
    assert!(
        prompt.contains("Bash"),
        "should suggest Bash as a combination"
    );
}

/// Full context produces a comprehensive prompt.
#[tokio::test]
async fn test_read_generate_prompt_full_context() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let full_ctx = PromptGenerationContext {
        agent_id: "agent-1".into(),
        workdir: Some(WorkdirContext {
            path: "/home/user/project".into(),
            has_git: true,
            branch: Some("feat/x".into()),
            recent_changes: 7,
        }),
        available_tool_names: vec!["Read".into(), "Write".into(), "Bash".into()],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
        agent_role: None,
        agent_type: None,
    };
    let prompt = tool.generate_prompt(&full_ctx);
    assert!(prompt.contains("/home/user/project"));
    assert!(prompt.contains("feat/x"));
    assert!(prompt.contains("uncommitted change"));
    assert!(prompt.contains("Combine with"));
}

// ---------------------------------------------------------------------------
// Edge cases — missing arguments
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_missing_path_arg() {
    let tool = ReadTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let result = tool.call(serde_json::json!({}), &make_ctx("a")).await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

#[tokio::test]
async fn test_write_missing_content_arg() {
    let tool = WriteTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let result = tool
        .call(serde_json::json!({ "path": "/tmp/x" }), &make_ctx("a"))
        .await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

#[tokio::test]
async fn test_edit_missing_old_text_arg() {
    let tool = EditTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let result = tool
        .call(serde_json::json!({ "path": "/tmp/x" }), &make_ctx("a"))
        .await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

#[tokio::test]
async fn test_grep_missing_pattern_arg() {
    let tool = GrepTool::new(make_engine(vec![]), make_sm(), make_cm(), make_af());
    let result = tool.call(serde_json::json!({}), &make_ctx("a")).await;
    assert!(matches!(result, Err(ToolCallError::InvalidArgs(_))));
}

// ---------------------------------------------------------------------------
// ConfigWrite dimension tests
// ---------------------------------------------------------------------------

/// Create a PermissionEngine with a custom data_root so that
/// `is_config_file_path` recognizes paths under that root as config files.
fn make_engine_with_data_root(rules: Vec<Rule>, data_root: &std::path::Path) -> PermEngine {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Deny,
            file_read: Effect::Deny,
            file_write: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    Arc::new(tokio::sync::RwLock::new(PermissionEngine::new(
        rs,
        data_root.to_path_buf(),
    )))
}

/// Bundled PermDeps for testing the three-level check flow.
fn make_file_deps(
    rules: Vec<Rule>,
    data_root: &std::path::Path,
) -> crate::permission_check::PermDeps {
    (
        make_engine_with_data_root(rules, data_root),
        make_sm(),
        make_cm(),
        make_af_deny(),
    )
}

/// Config file write is intercepted by ConfigWrite dimension (forced deny).
/// The engine's config_write_forced_deny guard converts Allow → Denied,
/// so even with a FileOp allow rule, the ConfigWrite check returns Denied.
#[tokio::test]
async fn test_config_write_intercepted_by_config_write_dimension() {
    let cm = make_cm();
    let data_root = cm.config_dir().to_path_buf();
    // Path inside data_root but outside workspaces → is_config_file = true
    let config_path = data_root.join("agents/a1/test.json");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", &format!("{}/**", data_root.display()), "write"),
    ];
    let deps = make_file_deps(rules, &data_root);
    let ctx = make_ctx("a");

    // Write via check_and_execute — ConfigWrite dimension should intercept
    let result = check_and_execute(
        &deps,
        &ctx,
        &config_path.to_string_lossy(),
        "write",
        write_file(&config_path.to_string_lossy(), "content"),
    )
    .await;
    assert!(
        result.is_err(),
        "config file write should be denied by ConfigWrite dimension"
    );
    // File should NOT be written — ConfigWrite check blocks before I/O
    assert!(!config_path.exists());
}

/// Regular file write is NOT affected by ConfigWrite check.
/// The path is outside data_root, so is_config_file returns false
/// and the ConfigWrite check is skipped entirely.
#[tokio::test]
async fn test_regular_write_not_affected_by_config_write_check() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("output.txt");
    let cm = make_cm();
    let data_root = cm.config_dir().to_path_buf();
    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let deps = make_file_deps(rules, &data_root);
    let ctx = make_ctx("a");

    let result = check_and_execute(
        &deps,
        &ctx,
        &path.to_string_lossy(),
        "write",
        write_file(&path.to_string_lossy(), "hello"),
    )
    .await;
    assert!(result.is_ok(), "regular file write should succeed");
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "hello");
}

/// Config file write with explicit ConfigWrite Allow rule + FileWrite Allow rule.
/// ConfigWrite forced deny guard overrides the Allow rule, so the write is blocked.
/// This verifies the guard is applied at the engine level, not just at the tool level.
#[tokio::test]
async fn test_config_write_explicit_allow_still_denied_by_guard() {
    let cm = make_cm();
    let data_root = cm.config_dir().to_path_buf();
    let config_path = data_root.join("agents/a1/models.json");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", &format!("{}/**", data_root.display()), "write"),
        // Explicit ConfigWrite allow rule — guard should still override it
        allow_config_write_rule("a"),
    ];
    let deps = make_file_deps(rules, &data_root);
    let ctx = make_ctx("a");

    let result = check_and_execute(
        &deps,
        &ctx,
        &config_path.to_string_lossy(),
        "write",
        write_file(&config_path.to_string_lossy(), "{\"key\": \"value\"}"),
    )
    .await;
    assert!(
        result.is_err(),
        "config write with explicit allow rule should be denied by forced deny guard"
    );
    assert!(!config_path.exists(), "config file should not be written");
}

// ---------------------------------------------------------------------------
// Step 1.8: Integration tests — EditTool edits[] array, replace_all,
//           error paths, and WriteTool end-to-end behavior.
// ---------------------------------------------------------------------------

/// EditTool accepts an 'edits' array and applies multiple replacements
/// to the same file in one call (non-incremental, reverse-order).
#[tokio::test]
async fn test_edit_with_edits_array() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("multi.txt");
    std::fs::write(&path, "aaa bbb ccc").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = EditTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "edits": [
            { "oldText": "aaa", "newText": "AAA" },
            { "oldText": "ccc", "newText": "CCC" }
        ]
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "AAA bbb CCC");
}

/// EditTool legacy `oldText`/`newText` format still works (backward compat).
#[tokio::test]
async fn test_edit_backward_compat_old_new_text() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("legacy.txt");
    std::fs::write(&path, "before middle after").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = EditTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "oldText": "middle",
        "newText": "CENTER"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "before CENTER after");
}

/// EditTool `replace_all` flag replaces every occurrence of the old text.
#[tokio::test]
async fn test_edit_replace_all() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("replace_all.txt");
    std::fs::write(&path, "foo bar foo baz foo").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = EditTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "edits": [
            { "oldText": "foo", "newText": "FOO" }
        ],
        "replace_all": true
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "FOO bar FOO baz FOO");
}

/// EditTool returns ExecutionFailed when oldText is not found.
#[tokio::test]
async fn test_edit_old_text_not_found() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("notfound.txt");
    std::fs::write(&path, "hello world").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = EditTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "oldText": "nonexistent",
        "newText": "replacement"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("not found"),
                "error should mention 'not found': {msg}"
            );
        }
        other => panic!("expected ExecutionFailed, got: {other:?}"),
    }
}

/// WriteTool creates a new file when it doesn't exist.
#[tokio::test]
async fn test_write_new_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("newfile.txt");
    assert!(!path.exists());

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = WriteTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "created"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "created");
}

/// WriteTool overwrites an existing file.
#[tokio::test]
async fn test_write_overwrite_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("overwrite.txt");
    std::fs::write(&path, "original").unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "write"),
    ];
    let tool = WriteTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": path.to_str().unwrap(),
        "content": "replaced"
    });
    let result = tool.call(args, &make_ctx("a")).await;
    assert!(result.is_ok());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "replaced");
}
