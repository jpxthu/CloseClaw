//! Unit tests for `SessionsSpawnTool::call`.
//!
//! Covers two early-error scenarios defined in the plan (Step 1.8.B):
//! 1. Missing `task` argument → `ToolCallError::InvalidArgs`
//! 2. `ToolContext.session_id` is `None` → `ToolCallError::ExecutionFailed`
//!
//! These tests don't need a fully-valid agent config because the tool
//! short-circuits before invoking the spawn controller. We still build
//! real `SpawnController` + `SessionManager` so the construction path
//! matches the production wiring.

use std::sync::Arc;

use serde_json::json;

use crate::agent::spawn::SpawnController;
use crate::config::ConfigManager;
use crate::gateway::session_manager::SessionManager;
use crate::gateway::{DmScope, GatewayConfig};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::tools::builtin::sessions_spawn::SessionsSpawnTool;
use crate::tools::{Tool, ToolCallError, ToolContext};

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

/// Build a `SessionsSpawnTool` with minimal controller/manager wiring.
fn make_tool() -> SessionsSpawnTool {
    let cm = Arc::new(make_config_manager());
    let sm = Arc::new(make_session_manager());
    let controller = Arc::new(SpawnController::new(cm, sm.clone()));
    SessionsSpawnTool::new(controller, sm)
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
