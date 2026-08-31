//! Step 1.3 — Unit tests for graceful_stop with pending tool calls
//! and busy count child session lifecycle.
//!
//! Covers the four behavior dimensions from the plan:
//! 1. Pending tool calls without running tools → Completed
//! 2. Pending tool calls with running tools → continues waiting
//! 3. LLM stream not ended (timeout path) → TimedOut
//! 4. Busy count child session lifecycle: +1 on create, -1 on completion

use super::super::session_handles::GracefulStopResult;
use super::super::*;
use crate::run_health::TranscriptOp;
use closeclaw_common::ContentBlock;
use closeclaw_common::ShutdownSignal;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

// ── test doubles ─────────────────────────────────────────────────────────

/// Minimal mock implementing `ShutdownSignal` for graceful stop tests.
struct GracefulStopMock {
    shutting_down: AtomicBool,
    forceful: AtomicBool,
    busy: AtomicUsize,
}

impl GracefulStopMock {
    fn new() -> Self {
        Self {
            shutting_down: AtomicBool::new(false),
            forceful: AtomicBool::new(false),
            busy: AtomicUsize::new(0),
        }
    }
}

impl ShutdownSignal for GracefulStopMock {
    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }
    fn increment_busy(&self) {
        self.busy.fetch_add(1, Ordering::SeqCst);
    }
    fn decrement_busy(&self) {
        self.busy.fetch_sub(1, Ordering::SeqCst);
    }
    fn busy_count(&self) -> usize {
        self.busy.load(Ordering::SeqCst)
    }
    fn escalate_to_forceful(&self) -> bool {
        self.forceful.store(true, Ordering::SeqCst);
        true
    }
    fn is_forceful(&self) -> bool {
        self.forceful.load(Ordering::SeqCst)
    }
    fn drain_status(&self) -> closeclaw_common::DrainStatus {
        closeclaw_common::DrainStatus {
            state: closeclaw_common::shutdown::ShutdownState::Running,
            busy_count: self.busy.load(Ordering::SeqCst),
            is_draining: false,
            pending_items: Vec::new(),
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────

fn make_session(id: &str) -> Arc<RwLock<ConversationSession>> {
    Arc::new(RwLock::new(ConversationSession::new(
        id.to_string(),
        "gpt-4o".to_string(),
        tmp_path(),
    )))
}

fn assistant_msg_with_tool_use(call_id: &str, tool_name: &str) -> SessionMessage {
    SessionMessage {
        role: "assistant".to_string(),
        content_blocks: vec![
            ContentBlock::Text(format!("Using {}.", tool_name)),
            ContentBlock::ToolUse {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                input: "{}".to_string(),
            },
        ],
        timestamp: chrono::Utc::now(),
    }
}

fn user_msg(text: &str) -> SessionMessage {
    SessionMessage {
        role: "user".to_string(),
        content_blocks: vec![ContentBlock::Text(text.to_string())],
        timestamp: chrono::Utc::now(),
    }
}

// ── Test 1: Pending tool calls without running tools → Completed ─────────

/// When LLM stream has ended and there are pending tool calls (ToolUse
/// blocks in the last assistant message) but no running tools in
/// tool_states, `graceful_stop()` must return `Completed` — NOT
/// `TimedOut`. This is the fix for the infinite loop bug.
#[tokio::test]
async fn test_graceful_stop_pending_tool_calls_no_running_tools_returns_completed() {
    let cs = make_session("s_pending_no_running");

    // Set LLM to Idle (stream has ended).
    cs.read().await.set_llm_state(LlmState::Idle);

    // Add an assistant message with 2 ToolUse blocks (pending tool calls).
    {
        let mut guard = cs.write().await;
        guard.apply_transcript_op(
            TranscriptOp::Rewrite,
            vec![
                user_msg("do something"),
                SessionMessage {
                    role: "assistant".to_string(),
                    content_blocks: vec![
                        ContentBlock::Text("Using bash and grep.".to_string()),
                        ContentBlock::ToolUse {
                            id: "call-pending-1".to_string(),
                            name: "bash".to_string(),
                            input: "{}".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: "call-pending-2".to_string(),
                            name: "grep".to_string(),
                            input: "{}".to_string(),
                        },
                    ],
                    timestamp: chrono::Utc::now(),
                },
            ],
        );
    }

    // Verify pending tool calls exist.
    let pending = cs.read().await.extract_pending_tool_calls();
    assert_eq!(pending.len(), 2, "must have 2 pending tool calls");

    // Verify no running tools (check tool_states directly).
    let guard = cs.read().await;
    let tool_states = guard.tool_states.read().expect("lock");
    assert!(tool_states.is_empty(), "must have no running tools");
    drop(tool_states);
    drop(guard);

    // graceful_stop must return Completed (not TimedOut or infinite loop).
    let result = cs
        .read()
        .await
        .graceful_stop(Duration::from_secs(10), None)
        .await;
    assert_eq!(
        result,
        GracefulStopResult::Completed,
        "pending tools without running tools must return Completed"
    );
}

// ── Test 2: Pending tool calls with running tools → continues waiting ────

/// When LLM stream has ended and there are both pending tool calls
/// (ToolUse in messages) AND running tools (in tool_states),
/// `graceful_stop()` must continue waiting for the running tools.
#[tokio::test]
async fn test_graceful_stop_pending_tool_calls_with_running_tools_waits() {
    let cs = make_session("s_pending_with_running");

    // Set LLM to Idle (stream has ended).
    cs.read().await.set_llm_state(LlmState::Idle);

    // Add an assistant message with ToolUse blocks.
    {
        let mut guard = cs.write().await;
        guard.apply_transcript_op(
            TranscriptOp::Rewrite,
            vec![
                user_msg("do something"),
                assistant_msg_with_tool_use("call-wait", "bash"),
            ],
        );
    }

    // Register a running tool (simulates tool in progress).
    {
        let guard = cs.read().await;
        guard.register_tool_call("running-tool", "bash", "long cmd");
        guard.update_tool_state(
            "running-tool",
            closeclaw_common::ToolExecState::RunningForeground,
        );
    }

    let guard = cs.read().await;
    let tool_states = guard.tool_states.read().expect("lock");
    assert!(!tool_states.is_empty(), "must have running tools");
    drop(tool_states);
    drop(guard);

    // grace_timeout short enough that it times out (running tool never completes).
    let result = cs
        .read()
        .await
        .graceful_stop(Duration::from_millis(200), None)
        .await;
    assert_eq!(
        result,
        GracefulStopResult::TimedOut,
        "running tools prevent immediate completion; must time out"
    );
}

// ── Test 3: Timeout path (LLM stream not ended) → TimedOut ──────────────

/// When the LLM stream is still active (Receiving/Requesting),
/// `graceful_stop()` must continue polling until timeout.
#[tokio::test]
async fn test_graceful_stop_streaming_not_ended_times_out() {
    let cs = make_session("s_streaming_timeout");

    // Set LLM to Receiving (stream is active).
    cs.read().await.set_llm_state(LlmState::Receiving);

    let result = cs
        .read()
        .await
        .graceful_stop(Duration::from_millis(200), None)
        .await;
    assert_eq!(
        result,
        GracefulStopResult::TimedOut,
        "active LLM stream must cause timeout"
    );
}

/// When the LLM stream transitions from Requesting to Idle mid-wait,
/// `graceful_stop()` returns `Completed` if no tools are running.
#[tokio::test]
async fn test_graceful_stop_streaming_then_idle_returns_completed() {
    let cs = make_session("s_streaming_then_idle");

    // Set LLM to Requesting (stream is active).
    cs.read().await.set_llm_state(LlmState::Requesting);

    // Spawn a task to set LLM to Idle after a short delay.
    let cs_clone = Arc::clone(&cs);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cs_clone.read().await.set_llm_state(LlmState::Idle);
    });

    // generous timeout; should complete once stream ends.
    let result = cs
        .read()
        .await
        .graceful_stop(Duration::from_secs(5), None)
        .await;
    assert_eq!(
        result,
        GracefulStopResult::Completed,
        "LLM stream ended + no running tools must return Completed"
    );
}

// ── Test 4: Busy count child session lifecycle ───────────────────────────

/// Verifies that register_child_handle increments busy_count by 1
/// and unregister_child_handle decrements it by 1, resulting in a
/// net-zero change after a full lifecycle.
#[tokio::test]
async fn test_busy_count_child_session_lifecycle_net_zero() {
    let parent = make_session("s_busy_lifecycle");
    let sh = Arc::new(GracefulStopMock::new());
    {
        let mut guard = parent.write().await;
        guard.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }

    let child = make_session("child_lifecycle");
    assert_eq!(sh.busy_count(), 0, "initial busy count must be 0");

    // Create child: register_child_handle increments busy count.
    parent
        .read()
        .await
        .register_child_handle("child_lifecycle", Arc::downgrade(&child));
    assert_eq!(sh.busy_count(), 1, "busy count must be 1 after register");

    // Complete child: unregister_child_handle decrements busy count.
    parent
        .read()
        .await
        .unregister_child_handle("child_lifecycle");
    assert_eq!(
        sh.busy_count(),
        0,
        "busy count must return to 0 after unregister"
    );
}

/// Multiple children: busy count tracks net registrations correctly.
#[tokio::test]
async fn test_busy_count_multiple_children() {
    let parent = make_session("s_busy_multi");
    let sh = Arc::new(GracefulStopMock::new());
    {
        let mut guard = parent.write().await;
        guard.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }

    let c1 = make_session("mc1");
    let c2 = make_session("mc2");
    let c3 = make_session("mc3");

    parent
        .read()
        .await
        .register_child_handle("mc1", Arc::downgrade(&c1));
    assert_eq!(sh.busy_count(), 1);
    parent
        .read()
        .await
        .register_child_handle("mc2", Arc::downgrade(&c2));
    assert_eq!(sh.busy_count(), 2);
    parent
        .read()
        .await
        .register_child_handle("mc3", Arc::downgrade(&c3));
    assert_eq!(sh.busy_count(), 3);

    // Complete children one by one.
    parent.read().await.unregister_child_handle("mc1");
    assert_eq!(sh.busy_count(), 2);
    parent.read().await.unregister_child_handle("mc2");
    assert_eq!(sh.busy_count(), 1);
    parent.read().await.unregister_child_handle("mc3");
    assert_eq!(sh.busy_count(), 0);
}

/// Busy count through full stop lifecycle: register → stop → count resets.
#[tokio::test]
async fn test_busy_count_through_stop_resets() {
    use closeclaw_common::shutdown::ShutdownMode;

    let parent = make_session("s_busy_stop");
    let sh = Arc::new(GracefulStopMock::new());
    {
        let mut guard = parent.write().await;
        guard.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }

    let child = make_session("child_stop");
    parent
        .read()
        .await
        .register_child_handle("child_stop", Arc::downgrade(&child));
    assert_eq!(sh.busy_count(), 1);

    parent
        .read()
        .await
        .stop(true, ShutdownMode::Forceful, Duration::ZERO)
        .await;
    assert_eq!(
        sh.busy_count(),
        0,
        "stop must clear handles and reset busy count"
    );
}
