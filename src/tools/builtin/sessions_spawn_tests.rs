//! Unit tests for `SessionsSpawnTool::call`.
//!
//! Covers two early-error scenarios defined in the plan (Step 1.8.B):
//! 1. Missing `task` argument → `ToolCallError::InvalidArgs`
//! 2. `ToolContext.session_id` is `None` → `ToolCallError::ExecutionFailed`
//!
//! Plus a schema validation test for the `fork` parameter.
//!
//! These tests don't need a fully-valid agent config because the tool
//! short-circuits before invoking the spawn controller. We still build
//! real `SpawnController` + `SessionManager` so the construction path
//! matches the production wiring.

use serde_json::json;

use crate::agent::spawn::SpawnController;
use crate::config::ConfigManager;
use crate::gateway::session_manager::SessionManager;
use crate::gateway::{DmScope, GatewayConfig};
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::rules::RuleSetBuilder;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::tools::builtin::sessions_spawn::SessionsSpawnTool;
use crate::tools::{Tool, ToolCallError, ToolContext};

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal `GatewayConfig` for tests.
fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

/// Build a minimal `SessionManager` (no storage, no workspace).
fn make_session_manager() -> SessionManager {
    SessionManager::new(
        &test_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    )
}

/// Build a `ConfigManager` over a tempdir. We don't call `load()` because
/// the test never reaches the agent-lookup step in the spawn tool.
fn make_config_manager() -> ConfigManager {
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed")
}

/// Build a `PermissionEngine` with an empty RuleSet.
fn make_permission_engine() -> PermissionEngine {
    PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap())
}

fn make_tool() -> SessionsSpawnTool {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = Arc::new(SpawnController::new(cm.clone(), sm.clone()));
    let pe = Arc::new(make_permission_engine());
    let ar = Arc::new(crate::agent::registry::AgentRegistry::new());
    SessionsSpawnTool::new(controller, sm, pe, cm, ar)
}

/// A `ToolContext` with `session_id = None` — used to exercise the
/// "no parent session" branch.
fn ctx_without_session() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    }
}

/// A `ToolContext` with a `session_id` set — used for completeness even
/// though the tests in this file don't reach the parent-session lookup.
fn ctx_with_session() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: Some("parent-session".to_string()),
        call_id: None,
        session: None,
    }
}

// ---------------------------------------------------------------------------
// 1. test_sessions_spawn_missing_task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sessions_spawn_missing_task() {
    let tool = make_tool();

    // Empty args: no `task` field.
    let result = tool.call(json!({}), &ctx_with_session()).await;

    let err = result.expect_err("call should fail when `task` is missing");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("task"),
                "error message should mention 'task', got: {}",
                msg
            );
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 2. test_sessions_spawn_no_session_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sessions_spawn_no_session_id() {
    let tool = make_tool();

    // Task is provided, but ToolContext.session_id is None.
    let result = tool
        .call(json!({"task": "do something"}), &ctx_without_session())
        .await;

    let err = result.expect_err("call should fail when session_id is None");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("session_id"),
                "error message should mention 'session_id', got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 3. test_sessions_spawn_fork_schema
// ---------------------------------------------------------------------------

#[test]
fn test_sessions_spawn_fork_schema() {
    let tool = make_tool();
    let schema = tool.input_schema();

    let fork_prop = schema["properties"]["fork"]
        .as_object()
        .expect("fork should be an object in properties");

    assert_eq!(fork_prop["type"], "boolean");
    assert_eq!(fork_prop["default"], false);
    assert!(
        fork_prop["description"].as_str().unwrap().contains("fork"),
        "description should mention fork"
    );
}

// ---------------------------------------------------------------------------
// 4. test_sessions_spawn_allowed_tools_schema
// ---------------------------------------------------------------------------

#[test]
fn test_sessions_spawn_allowed_tools_schema() {
    let tool = make_tool();
    let schema = tool.input_schema();

    let at_prop = schema["properties"]["allowedTools"]
        .as_object()
        .expect("allowedTools should be an object in properties");

    assert_eq!(at_prop["type"], "array");
    let items = at_prop["items"]
        .as_object()
        .expect("items should be an object");
    assert_eq!(items["type"], "string");
    assert!(
        at_prop["description"]
            .as_str()
            .unwrap()
            .contains("whitelist"),
        "description should mention whitelist"
    );
}

// ---------------------------------------------------------------------------
// 5. test_sessions_spawn_missing_task_error_with_allowed_tools
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sessions_spawn_missing_task_error_with_allowed_tools() {
    let tool = make_tool();
    // allowedTools present but task is missing — should still fail
    let result = tool
        .call(json!({"allowedTools": ["ReadTool"]}), &ctx_with_session())
        .await;
    let err = result.expect_err("should fail without task");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("task"), "error should mention 'task'");
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

// ===========================================================================
// Model parameter parsing tests (Step 1.4)
// ===========================================================================

// ---------------------------------------------------------------------------
// 6. test_sessions_spawn_model_param_parsed
// ---------------------------------------------------------------------------

/// When `model` is present in the JSON args, `parse_args` should extract
/// it as `Some(String)` in `SpawnArgs.model`.
#[test]
fn test_sessions_spawn_model_param_parsed() {
    let args = json!({
        "task": "do work",
        "model": "deepseek/deepseek-chat"
    });
    let spawn_args = crate::tools::builtin::sessions_spawn::SessionsSpawnTool::parse_args(&args)
        .expect("parse_args should succeed");
    assert_eq!(
        spawn_args.model.as_deref(),
        Some("deepseek/deepseek-chat"),
        "model should be parsed from args"
    );
}

// ---------------------------------------------------------------------------
// 7. test_sessions_spawn_model_param_absent
// ---------------------------------------------------------------------------

/// When `model` is absent from the JSON args, `parse_args` should set
/// `SpawnArgs.model` to `None`.
#[test]
fn test_sessions_spawn_model_param_absent() {
    let args = json!({"task": "do work"});
    let spawn_args = crate::tools::builtin::sessions_spawn::SessionsSpawnTool::parse_args(&args)
        .expect("parse_args should succeed");
    assert!(
        spawn_args.model.is_none(),
        "model should be None when not provided"
    );
}

// ---------------------------------------------------------------------------
// 8. test_sessions_spawn_model_param_empty_string
// ---------------------------------------------------------------------------

/// When `model` is present but empty string, `parse_args` should set
/// `SpawnArgs.model` to `Some("")` (empty string is still a value).
#[test]
fn test_sessions_spawn_model_param_empty_string() {
    let args = json!({"task": "do work", "model": ""});
    let spawn_args = crate::tools::builtin::sessions_spawn::SessionsSpawnTool::parse_args(&args)
        .expect("parse_args should succeed");
    assert_eq!(
        spawn_args.model.as_deref(),
        Some(""),
        "model should be Some(\"\") when set to empty string"
    );
}

// ---------------------------------------------------------------------------
// 9. test_sessions_spawn_model_param_schema
// ---------------------------------------------------------------------------

/// The input schema should declare `model` as a string property.
#[test]
fn test_sessions_spawn_model_param_schema() {
    let tool = make_tool();
    let schema = tool.input_schema();
    let model_prop = schema["properties"]["model"]
        .as_object()
        .expect("model should be an object in properties");
    assert_eq!(model_prop["type"], "string");
    assert!(
        model_prop["description"]
            .as_str()
            .unwrap()
            .contains("Override"),
        "description should mention override"
    );
}
