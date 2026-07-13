//! Unit tests for `SessionsSpawnTool::call`.
//!
//! Covers two early-error scenarios defined in the plan (Step 1.8.B):
//! 1. Missing `task` argument → `ToolCallError::InvalidArgs`
//! 2. `ToolContext.session_id` is `None` → `ToolCallError::ExecutionFailed`
//!
//! Plus schema validation tests for the `fork` and `model` parameters.
//!
//! Tests requiring `SpawnController` / `AgentRegistry` (main-crate types)
//! are marked `#[ignore]` until moved to integration tests.

use serde_json::json;

use crate::builtin::sessions_spawn::SessionsSpawnTool;

// ---------------------------------------------------------------------------
// Model parameter parsing tests (Step 1.4) — no main-crate deps
// ---------------------------------------------------------------------------

/// When `model` is present in the JSON args, `parse_args` should extract
/// it as `Some(String)` in `SpawnArgs.model`.
#[test]
fn test_sessions_spawn_model_param_parsed() {
    let args = json!({
        "task": "do work",
        "model": "deepseek/deepseek-chat"
    });
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.model.as_deref(),
        Some("deepseek/deepseek-chat"),
        "model should be parsed from args"
    );
}

/// When `model` is absent from the JSON args, `parse_args` should set
/// `SpawnArgs.model` to `None`.
#[test]
fn test_sessions_spawn_model_param_absent() {
    let args = json!({"task": "do work"});
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert!(
        spawn_args.model.is_none(),
        "model should be None when not provided"
    );
}

/// When `model` is present but empty string, `parse_args` should set
/// `SpawnArgs.model` to `Some("")` (empty string is still a value).
#[test]
fn test_sessions_spawn_model_param_empty_string() {
    let args = json!({"task": "do work", "model": ""});
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.model.as_deref(),
        Some(""),
        "model should be Some(\"\") when set to empty string"
    );
}

// ---------------------------------------------------------------------------
// Timeout parameter parsing tests
// ---------------------------------------------------------------------------

/// When `timeout` is present in the JSON args, `parse_args` should extract
/// it as `Some(u64)` in `SpawnArgs.timeout`.
#[test]
fn test_sessions_spawn_timeout_param_parsed() {
    let args = json!({
        "task": "do work",
        "timeout": 120
    });
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.timeout,
        Some(120),
        "timeout should be parsed from args"
    );
}

/// When `timeout` is absent from the JSON args, `parse_args` should set
/// `SpawnArgs.timeout` to `None`.
#[test]
fn test_sessions_spawn_timeout_param_absent() {
    let args = json!({"task": "do work"});
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert!(
        spawn_args.timeout.is_none(),
        "timeout should be None when not provided"
    );
}

/// When `timeout` is 0, `parse_args` should pass through as `Some(0)`.
#[test]
fn test_sessions_spawn_timeout_param_zero() {
    let args = json!({"task": "do work", "timeout": 0});
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.timeout,
        Some(0),
        "timeout=0 should be parsed as Some(0)"
    );
}

// ---------------------------------------------------------------------------
// Label parameter parsing tests

/// When `label` is present in the JSON args, `parse_args` should extract
/// it as `Some(String)` in `SpawnArgs.label`.
#[test]
fn test_sessions_spawn_label_param_parsed() {
    let args = json!({
        "task": "do work",
        "label": "my-subtask"
    });
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.label.as_deref(),
        Some("my-subtask"),
        "label should be parsed from args"
    );
}

/// When `label` is absent from the JSON args, `parse_args` should set
/// `SpawnArgs.label` to `None`.
#[test]
fn test_sessions_spawn_label_param_absent() {
    let args = json!({"task": "do work"});
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert!(
        spawn_args.label.is_none(),
        "label should be None when not provided"
    );
}

/// When `label` is empty string, `parse_args` should set it to `Some("")`.
#[test]
fn test_sessions_spawn_label_param_empty_string() {
    let args = json!({"task": "do work", "label": ""});
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.label.as_deref(),
        Some(""),
        "label=\"\" should be parsed as Some(\"\")"
    );
}

// ---------------------------------------------------------------------------
// promptTemplate + allowedTools independence tests (Step 1.4)
// ---------------------------------------------------------------------------

/// promptTemplate and allowedTools are independent parameters.
/// Specifying promptTemplate must NOT inject or override allowedTools.
#[test]
fn test_prompt_template_does_not_affect_allowed_tools() {
    let args = json!({
        "task": "analyze code",
        "promptTemplate": "explore"
    });
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert!(
        spawn_args.allowed_tools.is_none(),
        "promptTemplate must not inject allowed_tools; got {:?}",
        spawn_args.allowed_tools
    );
}

/// Explicit allowedTools is parsed independently of promptTemplate.
#[test]
fn test_explicit_allowed_tools_with_prompt_template() {
    let args = json!({
        "task": "validate changes",
        "promptTemplate": "validation",
        "allowedTools": ["read", "exec"]
    });
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.allowed_tools,
        Some(vec!["read".to_string(), "exec".to_string()]),
        "explicit allowedTools must be preserved alongside promptTemplate"
    );
}

/// Explicit allowedTools works without promptTemplate.
#[test]
fn test_explicit_allowed_tools_without_prompt_template() {
    let args = json!({
        "task": "run tests",
        "allowedTools": ["bash", "read"]
    });
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert_eq!(
        spawn_args.allowed_tools,
        Some(vec!["bash".to_string(), "read".to_string()]),
        "explicit allowedTools must work without promptTemplate"
    );
}

/// Empty allowedTools array is treated as None (no override).
#[test]
fn test_empty_allowed_tools_treated_as_none() {
    let args = json!({
        "task": "do work",
        "allowedTools": []
    });
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert!(
        spawn_args.allowed_tools.is_none(),
        "empty allowedTools array should be treated as None"
    );
}

/// promptTemplate is parsed correctly as each valid variant.
#[test]
fn test_prompt_template_parsed_for_each_variant() {
    let variants = ["explore", "validation", "plan", "executor"];
    for variant in variants {
        let args = json!({"task": "test", "promptTemplate": variant});
        let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
        assert!(
            spawn_args.prompt_template.is_some(),
            "promptTemplate={} should be parsed successfully",
            variant
        );
    }
}

/// Invalid promptTemplate value is rejected.
#[test]
fn test_invalid_prompt_template_rejected() {
    let args = json!({"task": "test", "promptTemplate": "invalid"});
    let result = SessionsSpawnTool::parse_args(&args);
    assert!(result.is_err(), "invalid promptTemplate should be rejected");
}

/// promptTemplate absent means None (no prefix prepended).
#[test]
fn test_prompt_template_absent_means_none() {
    let args = json!({"task": "do work"});
    let spawn_args = SessionsSpawnTool::parse_args(&args).expect("parse_args should succeed");
    assert!(
        spawn_args.prompt_template.is_none(),
        "promptTemplate should be None when not provided"
    );
}

// ---------------------------------------------------------------------------
// Tests requiring SpawnController (main-crate type)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires SpawnController and AgentRegistry from main crate"]
async fn test_sessions_spawn_missing_task() {
    // Requires SpawnController from main crate.
}

#[tokio::test]
#[ignore = "requires SpawnController and AgentRegistry from main crate"]
async fn test_sessions_spawn_no_session_id() {
    // Requires SpawnController from main crate.
}

#[test]
#[ignore = "requires SpawnController and AgentRegistry from main crate"]
fn test_sessions_spawn_fork_schema() {
    // Requires make_tool() which needs SpawnController.
}

#[test]
#[ignore = "requires SpawnController and AgentRegistry from main crate"]
fn test_sessions_spawn_allowed_tools_schema() {
    // Requires make_tool() which needs SpawnController.
}

#[tokio::test]
#[ignore = "requires SpawnController and AgentRegistry from main crate"]
async fn test_sessions_spawn_missing_task_error_with_allowed_tools() {
    // Requires make_tool() which needs SpawnController.
}

#[test]
#[ignore = "requires SpawnController and AgentRegistry from main crate"]
fn test_sessions_spawn_model_param_schema() {
    // Requires make_tool() which needs SpawnController.
}
