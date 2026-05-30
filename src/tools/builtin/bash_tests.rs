//! Unit tests for BashTool (extracted from inline tests).
//!
//! These tests are compiled via `#[path = "bash_tests.rs"]` inside
//! `bash.rs`'s `#[cfg(test)] mod tests`, which grants access to
//! private items in the parent module.

use super::*;
use serde_json::json;

fn test_permission_engine() -> Arc<PermissionEngine> {
    use crate::permission::rules::RuleSetBuilder;
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

fn test_bg_manager() -> Arc<BackgroundTaskManager> {
    Arc::new(BackgroundTaskManager::new())
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
    let tool = BashTool::new(test_permission_engine(), test_bg_manager());
    assert_eq!(tool.name(), "Bash");
    assert_eq!(tool.group(), "bash");
}

#[test]
fn test_bash_tool_flags() {
    let tool = BashTool::new(test_permission_engine(), test_bg_manager());
    let flags = tool.flags();
    assert!(flags.is_destructive);
    assert!(flags.is_expensive);
}

// --- input_schema ---

#[test]
fn test_input_schema_command_required() {
    let tool = BashTool::new(test_permission_engine(), test_bg_manager());
    let schema = tool.input_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("command")));
}

#[test]
fn test_input_schema_six_properties() {
    let tool = BashTool::new(test_permission_engine(), test_bg_manager());
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
