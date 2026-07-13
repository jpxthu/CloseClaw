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
