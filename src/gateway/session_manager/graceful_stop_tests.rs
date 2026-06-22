//! Tests for the graceful-shutdown state machine in `stop_single_session`.
//!
//! Covers Step 1.2 of the graceful-shutdown plan: the three sub-states
//! (streaming, streaming-ended-with-tools, idle, tool-running) and
//! their transitions during a graceful stop.
//!
//! Note on the `streaming_seen` path: `stop_single_session` holds a
//! read-lock on the `Arc<RwLock<ConversationSession>>` for the entire
//! polling loop. A background task that tries to write to the same
//! session would deadlock.  Instead, tests for the streaming→idle
//! transition pre-set the **end-state** (LlmState::Idle + ToolUse in
//! the last assistant message) before calling `stop_all_sessions`.
//! The polling loop immediately sees `is_streaming == false`,
//! `streaming_seen == false`, `has_running_tools == false` and breaks.
//! The `extract_pending_tool_calls` logic is covered by the Step 1.1
//! unit tests; this file verifies the integration through
//! `stop_all_sessions`.

use crate::daemon::shutdown::ShutdownMode;
use crate::gateway::session_manager::test_helpers::{register_child_only, setup_parent_with_conv};
use crate::gateway::session_manager::SessionManager;
use crate::gateway::Session;
use crate::llm::session::{ChatSession, ConversationSession};
use crate::llm::session_state::{LlmState, ToolExecState};
use crate::llm::types::ContentBlock;
use crate::session::bootstrap::BootstrapMode;

use std::path::PathBuf;
use std::sync::Arc;

use super::spawn::SpawnMode;

// ── helpers ──────────────────────────────────────────────────────────────

fn make_test_session_manager() -> SessionManager {
    let config = crate::gateway::GatewayConfig::default();
    SessionManager::new(&config, None, None, BootstrapMode::Full, Default::default())
}

fn make_conversation_session(id: &str) -> Arc<tokio::sync::RwLock<ConversationSession>> {
    Arc::new(tokio::sync::RwLock::new(ConversationSession::new(
        id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    )))
}

/// Register a child session with a `ConversationSession` under the
/// given parent.  Both the `sessions` map and `conversation_sessions`
/// map are populated so `stop_all_sessions` can find and stop the
/// child.
async fn setup_child_with_conv(mgr: &SessionManager, parent_id: &str, child_id: &str) {
    register_child_only(mgr, parent_id, child_id, "worker", SpawnMode::Session).await;
    let cs = make_conversation_session(child_id);
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), cs);
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        Session {
            id: child_id.to_string(),
            agent_id: "worker".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );
}

/// Set the LLM state for a given session (via the `Arc`).
async fn set_llm_state(mgr: &SessionManager, session_id: &str, state: LlmState) {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let guard = cs.read().await;
    let mut llm = guard.llm_state.write().expect("llm_state lock poisoned");
    *llm = state;
}

/// Set a tool state for a given session (via the `Arc`).
async fn set_tool_state(
    mgr: &SessionManager,
    session_id: &str,
    tool_id: &str,
    state: ToolExecState,
) {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let guard = cs.read().await;
    let mut tools = guard
        .tool_states
        .write()
        .expect("tool_states lock poisoned");
    tools.insert(tool_id.to_string(), state);
}

/// Append an assistant message via `ChatSession::append_response`.
async fn append_assistant_message(
    mgr: &SessionManager,
    session_id: &str,
    blocks: Vec<ContentBlock>,
) {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let mut guard = cs.write().await;
    guard.append_response(crate::llm::types::UnifiedResponse {
        content_blocks: blocks,
        usage: crate::llm::types::UnifiedUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".to_string()),
    });
}

// ── Step 1.2: graceful stop scenarios ────────────────────────────────────

/// Session ends with ToolUse in the last assistant message →
/// `stop_all_sessions` should stop successfully and the child should
/// be removed from active sessions.
///
/// We pre-set the end-state (LlmState::Idle, assistant with ToolUse)
/// before calling `stop_all_sessions`.  The polling loop sees idle
/// immediately and breaks.  The `extract_pending_tool_calls` path is
/// exercised by the Step 1.1 unit tests.
#[tokio::test]
async fn test_graceful_stop_streaming_with_tool_calls() {
    let mgr = make_test_session_manager();
    let parent_id = "parent_gst_stream_tools";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child_gst_stream_tools";
    setup_child_with_conv(&mgr, parent_id, child_id).await;

    // Pre-set end-state: LLM idle, last assistant has ToolUse.
    set_llm_state(&mgr, child_id, LlmState::Idle).await;
    append_assistant_message(
        &mgr,
        child_id,
        vec![
            ContentBlock::Text("Let me check.".to_string()),
            ContentBlock::ToolUse {
                id: "call_gs_1".to_string(),
                name: "get_weather".to_string(),
                input: r#"{"city":"Tokyo"}"#.to_string(),
            },
        ],
    )
    .await;

    let result = mgr.stop_all_sessions(ShutdownMode::Graceful, None).await;
    assert!(
        result.succeeded >= 2,
        "expected >= 2 succeeded, got {:?}",
        result
    );
    assert!(
        !mgr.has_session(&child_id).await,
        "child should be removed after graceful stop"
    );
}

/// Streaming ends → assistant message has no ToolUse → normal stop.
#[tokio::test]
async fn test_graceful_stop_streaming_no_tool_calls() {
    let mgr = make_test_session_manager();
    let parent_id = "parent_gst_stream_notools";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child_gst_stream_notools";
    setup_child_with_conv(&mgr, parent_id, child_id).await;

    // Pre-set: LLM idle, assistant text only (no ToolUse).
    set_llm_state(&mgr, child_id, LlmState::Idle).await;
    append_assistant_message(
        &mgr,
        child_id,
        vec![ContentBlock::Text("Done.".to_string())],
    )
    .await;

    let result = mgr.stop_all_sessions(ShutdownMode::Graceful, None).await;
    assert!(result.succeeded >= 2);
    assert!(!mgr.has_session(&child_id).await);
}

/// Idle session → direct stop, pending_operations should be empty.
#[tokio::test]
async fn test_graceful_stop_idle() {
    let mgr = make_test_session_manager();
    let parent_id = "parent_gst_idle";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child_gst_idle";
    setup_child_with_conv(&mgr, parent_id, child_id).await;

    // Default: LlmState::Idle, no tools running.
    let result = mgr.stop_all_sessions(ShutdownMode::Graceful, None).await;
    assert!(result.succeeded >= 2);
    assert!(!mgr.has_session(&child_id).await);
}

/// Tool is running → wait for tool to finish → pending_operations empty.
///
/// Set a tool in RunningForeground state.  The polling loop sees
/// `has_running_tools == true` and enters the `continue` branch
/// (new Step 1.4 fix).  A background task clears the tool state
/// during the polling sleep, so the loop eventually sees
/// `has_running_tools == false` and breaks.
#[tokio::test]
async fn test_graceful_stop_tool_running() {
    let mgr = make_test_session_manager();
    let parent_id = "parent_gst_tool_running";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child_gst_tool_running";
    setup_child_with_conv(&mgr, parent_id, child_id).await;

    // LLM idle, tool in RunningForeground state (actively running).
    // The loop sees `has_running_tools == true` → continue (wait).
    // A background task clears the tool state so the loop can break.
    set_llm_state(&mgr, child_id, LlmState::Idle).await;
    set_tool_state(&mgr, child_id, "tool_1", ToolExecState::RunningForeground).await;

    // Spawn a task that clears the tool state after a brief delay,
    // simulating tool completion.  The polling loop's sleep releases
    // the read-lock, allowing this task to acquire it and write.
    let cs_arc = mgr
        .get_conversation_session(child_id)
        .await
        .expect("conversation session not found");
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let guard = cs_arc.read().await;
        let mut tools = guard
            .tool_states
            .write()
            .expect("tool_states lock poisoned");
        tools.remove("tool_1");
    });

    let result = mgr.stop_all_sessions(ShutdownMode::Graceful, None).await;
    assert!(result.succeeded >= 2);
    assert!(!mgr.has_session(&child_id).await);
}

/// Forceful mode should not be affected by the graceful state machine.
#[tokio::test]
async fn test_forceful_stop_unchanged() {
    let mgr = make_test_session_manager();
    let parent_id = "parent_gst_forceful";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child_gst_forceful";
    setup_child_with_conv(&mgr, parent_id, child_id).await;

    // Streaming + tool running — forceful should not wait.
    set_llm_state(&mgr, child_id, LlmState::Receiving).await;
    set_tool_state(&mgr, child_id, "tool_f", ToolExecState::RunningForeground).await;

    let start = tokio::time::Instant::now();
    let result = mgr.stop_all_sessions(ShutdownMode::Forceful, None).await;
    let elapsed = start.elapsed();

    assert!(result.succeeded >= 2);
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "forceful mode should complete quickly, took {:?}",
        elapsed
    );
}
