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

use crate::session_manager::test_helpers::{register_child_only, setup_parent_with_conv};
use crate::session_manager::SessionManager;
use crate::Session;
use closeclaw_common::shutdown::{ShutdownMode, ShutdownSignal};
use closeclaw_llm::session_state::{LlmState, ToolExecState};
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::llm_session::{ChatSession, ConversationSession};

use std::path::PathBuf;
use std::sync::Arc;

use super::spawn::SpawnMode;

// ── helpers ──────────────────────────────────────────────────────────────

fn make_test_session_manager() -> SessionManager {
    let config = crate::GatewayConfig::default();
    SessionManager::new(&config, None, None, Default::default())
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
    tools.insert(tool_id.to_string(), (state, None));
}

/// Append an assistant message via `ChatSession::append_response`.
async fn append_assistant_message(
    mgr: &SessionManager,
    session_id: &str,
    blocks: Vec<ContentBlock>,
) {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let mut guard = cs.write().await;
    guard.append_response(closeclaw_llm::types::UnifiedResponse {
        content_blocks: blocks,
        usage: closeclaw_llm::types::UnifiedUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".to_string()),
        retry_attempts: 0,
    });
}

/// A mock `ShutdownSignal` for escalation tests.
struct MockEscalationSignal {
    is_shutting_down: std::sync::atomic::AtomicBool,
    is_forceful: std::sync::atomic::AtomicBool,
}

impl closeclaw_common::shutdown::ShutdownSignal for MockEscalationSignal {
    fn is_shutting_down(&self) -> bool {
        self.is_shutting_down
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    fn increment_busy(&self) {}
    fn decrement_busy(&self) {}
    fn busy_count(&self) -> usize {
        0
    }
    fn escalate_to_forceful(&self) -> bool {
        self.is_forceful
            .store(true, std::sync::atomic::Ordering::SeqCst);
        true
    }
    fn is_forceful(&self) -> bool {
        self.is_forceful.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn drain_status(&self) -> closeclaw_common::DrainStatus {
        closeclaw_common::DrainStatus {
            state: if self
                .is_shutting_down
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                if self.is_forceful.load(std::sync::atomic::Ordering::SeqCst) {
                    closeclaw_common::ShutdownState::ForcefulShuttingDown
                } else {
                    closeclaw_common::ShutdownState::ShuttingDown
                }
            } else {
                closeclaw_common::ShutdownState::Running
            },
            busy_count: 0,
            is_draining: false,
        }
    }
}

/// Install a mock shutdown handle on a `SessionManager`.
async fn install_mock_handle(mgr: &SessionManager, is_forceful: bool) -> Arc<MockEscalationSignal> {
    let mock = Arc::new(MockEscalationSignal {
        is_shutting_down: std::sync::atomic::AtomicBool::new(true),
        is_forceful: std::sync::atomic::AtomicBool::new(is_forceful),
    });
    let handle = Arc::new(crate::shutdown_handle::ShutdownHandle::new(
        mock.clone() as Arc<dyn closeclaw_common::shutdown::ShutdownSignal>
    ));
    mgr.set_shutdown_handle(handle).await;
    mock
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

    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(
        result.succeeded >= 2,
        "expected >= 2 succeeded, got {:?}",
        result
    );
    // Step 1.1 removed remove_session from stop_single_session;
    // sessions stay in tracking tables until flush_all cleans them up.
    assert!(
        mgr.has_session(&child_id).await,
        "session should remain in tracking tables after stop"
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

    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(result.succeeded >= 2);
    // Sessions remain in tracking tables after stop (Step 1.1).
    assert!(mgr.has_session(&child_id).await);
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
    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(result.succeeded >= 2);
    // Sessions remain in tracking tables after stop (Step 1.1).
    assert!(mgr.has_session(&child_id).await);
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

    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(result.succeeded >= 2);
    // Sessions remain in tracking tables after stop (Step 1.1).
    assert!(mgr.has_session(&child_id).await);
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
    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Forceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    let elapsed = start.elapsed();

    assert!(result.succeeded >= 2);
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "forceful mode should complete quickly, took {:?}",
        elapsed
    );
}

// ── Step 1.1: escalation propagation tests ─────────────────────────────

/// Verify that the escalation detection in `stop_all_sessions` works:
/// when the shutdown handle reports forceful, `stop_all_sessions`
/// uses forceful mode even when called with Graceful.
///
/// This test verifies the core escalation detection mechanism. A
/// streaming session (LlmState::Receiving) would hang in graceful
/// mode forever. The forceful mock causes `stop_all_sessions` to
/// detect forceful and skip the graceful polling loop.
#[tokio::test]
async fn test_forceful_mock_stops_streaming_immediately() {
    let mgr = make_test_session_manager();
    let parent_id = "parent_forceful_mock";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child_forceful_mock";
    setup_child_with_conv(&mgr, parent_id, child_id).await;

    // Parent is streaming — would hang in graceful mode.
    set_llm_state(&mgr, parent_id, LlmState::Receiving).await;

    // Install a mock that reports forceful from the start.
    let _mock = install_mock_handle(&mgr, true).await;

    // Should complete quickly because mock is forceful.
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        mgr.stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        ),
    )
    .await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "forceful mock should not hang");
    let result = result.unwrap();
    assert!(
        result.succeeded >= 2,
        "expected >= 2 stopped, got {:?}",
        result
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "forceful mode should be fast, took {:?}",
        elapsed
    );
    // Sessions remain in tracking tables after stop (Step 1.1).
    assert!(mgr.has_session(&parent_id).await);
    assert!(mgr.has_session(&child_id).await);
}

/// Verify escalation propagation: after graceful→forceful escalation,
/// `stop_all_sessions` switches to forceful mode for remaining levels.
///
/// Setup: child → parent (two levels, leaf-to-root). The child is idle
/// (stops quickly in graceful). The parent is streaming (would block in
/// graceful forever). The mock starts as forceful (simulating escalation
/// that happened before stop_all_sessions was called).
///
/// Uses `tokio::time::timeout` to detect hangs — if escalation does
/// not propagate, the parent hangs and the test times out.
#[tokio::test]
async fn test_escalation_propagation_across_levels() {
    let mgr = make_test_session_manager();

    // ── set up two-level tree: child → parent ───────────────────────
    let parent_id = "parent_esc_prop";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child_esc_prop";
    setup_child_with_conv(&mgr, parent_id, child_id).await;

    // child: idle (will stop quickly in graceful mode).
    // parent: streaming — would block forever in graceful mode.
    set_llm_state(&mgr, parent_id, LlmState::Receiving).await;

    // Install forceful mock — simulates escalation happened before
    // stop_all_sessions was called.
    let _mock = install_mock_handle(&mgr, true).await;

    // ── run with timeout to detect hangs ─────────────────────────────
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        mgr.stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        ),
    )
    .await;

    assert!(
        result.is_ok(),
        "escalation did not propagate: parent session hung in graceful mode (timeout)"
    );
    let result = result.unwrap();
    assert!(
        result.succeeded >= 2,
        "expected >= 2 sessions stopped, got {:?}",
        result
    );
    // Sessions remain in tracking tables after stop (Step 1.1).
    assert!(mgr.has_session(&parent_id).await);
    assert!(mgr.has_session(&child_id).await);
}

// ── Step 1.2: escalation + idle stop behavior ─────────────────────────

/// Forceful escalation during LLM streaming interrupts graceful wait
/// and force-stops the session (design doc: retry with forceful).
#[tokio::test]
async fn test_graceful_escalation_interrupts_streaming_info() {
    let mgr = make_test_session_manager();
    let parent_id = "parent-escalate-fields";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child-escalate-fields";
    setup_child_with_conv(&mgr, parent_id, child_id).await;
    set_llm_state(&mgr, child_id, LlmState::Receiving).await;

    let mock = install_mock_handle(&mgr, false).await;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        mock.escalate_to_forceful();
    });

    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(result.total() >= 1);
    assert!(
        result.succeeded >= 1,
        "escalation should force-stop the session successfully"
    );
}

/// Forceful escalation during tool running interrupts graceful wait
/// and force-stops the session (design doc: retry with forceful).
#[tokio::test]
async fn test_graceful_escalation_interrupts_tool_info() {
    let mgr = make_test_session_manager();
    let parent_id = "parent-escalate-tool-info";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child-escalate-tool-info";
    setup_child_with_conv(&mgr, parent_id, child_id).await;
    set_tool_state(
        &mgr,
        child_id,
        "long_tool",
        ToolExecState::RunningForeground,
    )
    .await;

    let mock = install_mock_handle(&mgr, false).await;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        mock.escalate_to_forceful();
    });

    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(result.total() >= 1);
    assert!(
        result.succeeded >= 1,
        "escalation should force-stop the session successfully"
    );
}

/// Idle session stops immediately.
#[tokio::test]
async fn test_graceful_idle_no_timeout_info() {
    let mgr = make_test_session_manager();
    setup_parent_with_conv(&mgr, "parent-idle-no-timeout").await;
    let result = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(result.succeeded >= 1);
}

// ── Escalation interrupt tests ─────────────────────────────────────────

/// Forceful escalation interrupts graceful wait for streaming session
/// and force-stops the session.
#[tokio::test]
async fn test_graceful_escalation_interrupts_streaming() {
    let mgr = make_test_session_manager();
    let parent_id = "parent-escalate-stream";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child-escalate-stream";
    setup_child_with_conv(&mgr, parent_id, child_id).await;
    set_llm_state(&mgr, child_id, LlmState::Receiving).await;

    // Install mock starting as graceful
    let mock = install_mock_handle(&mgr, false).await;

    // Spawn escalation after 150ms
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        mock.escalate_to_forceful();
    });

    let r = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    // Session force-stopped due to escalation
    assert!(r.total() >= 1);
    assert!(
        r.succeeded >= 1,
        "escalation should force-stop the session successfully"
    );
}

/// Forceful escalation interrupts graceful wait for running tool
/// and force-stops the session.
#[tokio::test]
async fn test_graceful_escalation_interrupts_tool_running() {
    let mgr = make_test_session_manager();
    let parent_id = "parent-escalate-tool";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child-escalate-tool";
    setup_child_with_conv(&mgr, parent_id, child_id).await;
    set_tool_state(
        &mgr,
        child_id,
        "tool永不结束",
        ToolExecState::RunningForeground,
    )
    .await;

    let mock = install_mock_handle(&mgr, false).await;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        mock.escalate_to_forceful();
    });

    let r = mgr
        .stop_all_sessions(
            ShutdownMode::Graceful,
            std::time::Duration::from_secs(30),
            None,
        )
        .await;
    assert!(r.total() >= 1);
    assert!(
        r.succeeded >= 1,
        "escalation should force-stop the session successfully"
    );
}
