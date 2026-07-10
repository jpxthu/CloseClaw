//! Unit tests for file_ops tools — metadata and permission-check tests.

use super::*;
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

fn make_engine(rules: Vec<Rule>) -> PermEngine {
    let rs = RuleSetBuilder::new()
        .rules(rules)
        .defaults(Defaults {
            tool_call: Effect::Deny,
            file: Effect::Deny,
            ..Default::default()
        })
        .build()
        .unwrap();
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rs),
    ))
}

fn make_sm() -> SessionMgr {
    use closeclaw_gateway::GatewayConfig;
    use closeclaw_session::bootstrap::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;
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

fn make_cm() -> ConfigMgr {
    let tmp = TempDir::new().unwrap();
    Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    )
}

fn make_af() -> ApprovalMtx {
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

fn make_ctx(agent: &str) -> ToolContext {
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

fn allow_tool(agent: &str, skill: &str) -> Rule {
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

fn allow_file(agent: &str, path_glob: &str, op: &str) -> Rule {
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
    assert!(props.contains_key("oldText"));
    assert!(props.contains_key("newText"));
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
    assert_eq!(result.unwrap().data["content"], "hello");
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
