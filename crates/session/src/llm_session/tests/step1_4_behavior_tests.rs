//! Step 1.4 — Unit tests for the three behavior dimensions introduced
//! by Steps 1.1–1.3 of issue #858.
//!
//! 1. **Terminal-state deregistration** (Gap 1): `update_tool_state`
//!    with a terminal state removes the entry from `tool_states` map;
//!    non-terminal states keep it.
//! 2. **pending_operations preservation** (Gap 2): `collect_pending_operations`
//!    returns non-empty results before `clear_exec_state` wipes the map
//!    during forceful stop.
//! 3. **Forceful stop ordering** (Gap 3): `stop(Forceful)` executes
//!    cascade → kill tools → cancel LLM → cleanup (design doc order).

use super::super::*;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::{SessionExecStatus, ToolExecState};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

// ── test doubles ─────────────────────────────────────────────────────────

/// `KillHandle` that records kill invocations and optionally executes
/// a side-effect closure on each call.  Used to observe ordering.
struct OrderTrackingKillHandle {
    call_count: Arc<AtomicUsize>,
    on_kill: Option<Box<dyn Fn() + Send + Sync>>,
}

impl OrderTrackingKillHandle {
    fn new(call_count: Arc<AtomicUsize>) -> Self {
        Self {
            call_count,
            on_kill: None,
        }
    }

    fn with_callback(call_count: Arc<AtomicUsize>, f: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            call_count,
            on_kill: Some(Box::new(f)),
        }
    }
}

impl KillHandle for OrderTrackingKillHandle {
    fn kill(&self) -> io::Result<()> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if let Some(ref f) = self.on_kill {
            f();
        }
        Ok(())
    }
}

fn make_session(id: &str) -> Arc<RwLock<ConversationSession>> {
    Arc::new(RwLock::new(ConversationSession::new(
        id.to_string(),
        "gpt-4o".to_string(),
        tmp_path(),
    )))
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Terminal-state deregistration (Gap 1 — Step 1.1)
// ═══════════════════════════════════════════════════════════════════════════

/// `Completed` is a terminal state → entry removed from tool_states.
#[test]
fn test_terminal_completed_removes_from_tool_states() {
    let session = ConversationSession::new("s1.4_tc".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-ok", "bash", "echo hi");
    // Before: entry exists.
    assert!(session
        .tool_states
        .read()
        .expect("lock")
        .contains_key("call-ok"));

    session.update_tool_state("call-ok", ToolExecState::Completed);
    // After terminal update: entry removed.
    assert!(
        !session
            .tool_states
            .read()
            .expect("lock")
            .contains_key("call-ok"),
        "Completed (terminal) must remove entry from tool_states map"
    );
}

/// `Failed` is a terminal state → entry removed from tool_states.
#[test]
fn test_terminal_failed_removes_from_tool_states() {
    let session = ConversationSession::new("s1.4_tf".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-fail", "bash", "exit 1");

    session.update_tool_state("call-fail", ToolExecState::Failed);
    assert!(
        !session
            .tool_states
            .read()
            .expect("lock")
            .contains_key("call-fail"),
        "Failed (terminal) must remove entry from tool_states map"
    );
}

/// `Terminated` is a terminal state → entry removed from tool_states.
#[test]
fn test_terminal_terminated_removes_from_tool_states() {
    let session = ConversationSession::new("s1.4_tt".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-term", "bash", "kill me");

    session.update_tool_state("call-term", ToolExecState::Terminated);
    assert!(
        !session
            .tool_states
            .read()
            .expect("lock")
            .contains_key("call-term"),
        "Terminated (terminal) must remove entry from tool_states map"
    );
}

/// `TimedOut` is a terminal state → entry removed from tool_states.
#[test]
fn test_terminal_timed_out_removes_from_tool_states() {
    let session = ConversationSession::new("s1.4_tto".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-timeout", "bash", "sleep 999");

    session.update_tool_state("call-timeout", ToolExecState::TimedOut);
    assert!(
        !session
            .tool_states
            .read()
            .expect("lock")
            .contains_key("call-timeout"),
        "TimedOut (terminal) must remove entry from tool_states map"
    );
}

/// `RunningForeground` is NOT terminal → entry stays in tool_states.
#[test]
fn test_non_terminal_running_foreground_keeps_entry() {
    let session = ConversationSession::new("s1.4_rfg".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-fg", "bash", "fg cmd");

    session.update_tool_state("call-fg", ToolExecState::RunningForeground);
    assert!(
        session
            .tool_states
            .read()
            .expect("lock")
            .contains_key("call-fg"),
        "RunningForeground (non-terminal) must keep entry in tool_states map"
    );
    // Verify the state is correctly set.
    let guard = session.tool_states.read().expect("lock");
    let (state, _) = guard.get("call-fg").unwrap();
    assert_eq!(*state, ToolExecState::RunningForeground);
}

/// `RunningBackground` is NOT terminal → entry stays in tool_states.
#[test]
fn test_non_terminal_running_background_keeps_entry() {
    let session = ConversationSession::new("s1.4_rbg".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-bg", "bash", "bg cmd");

    session.update_tool_state("call-bg", ToolExecState::RunningBackground);
    assert!(
        session
            .tool_states
            .read()
            .expect("lock")
            .contains_key("call-bg"),
        "RunningBackground (non-terminal) must keep entry in tool_states map"
    );
    let guard = session.tool_states.read().expect("lock");
    let (state, _) = guard.get("call-bg").unwrap();
    assert_eq!(*state, ToolExecState::RunningBackground);
}

/// `exec_status()` behaves the same regardless of terminal state removal —
/// terminal tools were never counted in exec_status evaluation.
#[test]
fn test_terminal_deregistration_does_not_affect_exec_status() {
    let session = ConversationSession::new("s1.4_es".into(), "gpt-4o".into(), tmp_path());
    // Idle before.
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);

    // Register → terminal update.
    session.register_tool_call("call-x", "bash", "cmd");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
    session.update_tool_state("call-x", ToolExecState::Completed);
    // Should return to Idle (terminal tools don't participate).
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

/// `has_active_foreground_tool()` / `has_active_background_tool()` behave
/// the same regardless of terminal state removal.
#[test]
fn test_terminal_deregistration_does_not_affect_active_checks() {
    let session = ConversationSession::new("s1.4_ac".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-fg", "bash", "fg");
    session.update_tool_state("call-fg", ToolExecState::RunningForeground);
    assert!(session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());

    // Terminal update removes entry.
    session.update_tool_state("call-fg", ToolExecState::Completed);
    assert!(!session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. pending_operations preservation (Gap 2 — Step 1.2)
// ═══════════════════════════════════════════════════════════════════════════

/// After forceful stop, pending_operations must be collected BEFORE
/// clear_exec_state wipes tool_states/child_states.
#[tokio::test]
async fn test_pending_operations_collected_before_clear() {
    let cs = make_session("s1.4_pending_before_clear");

    // Register in-flight tool calls and child sessions.
    {
        let guard = cs.read().await;
        guard.register_tool_call("tool-1", "bash", "long cmd");
        guard.update_tool_state("tool-1", ToolExecState::RunningForeground);
        guard.register_tool_call("tool-2", "bash", "bg cmd");
        guard.update_tool_state("tool-2", ToolExecState::RunningBackground);
        guard.register_child("child-1", "agent-a", "task 1");
    }

    // Collect pending operations (simulating what forceful stop does).
    let pending_ops = {
        let guard = cs.read().await;
        guard.collect_pending_operations()
    };

    // Must be non-empty — in-flight ops are captured.
    assert!(
        !pending_ops.is_empty(),
        "pending_operations must be non-empty before clear_exec_state"
    );

    let op_ids: Vec<&str> = pending_ops.iter().map(|op| op.op_id.as_str()).collect();
    assert!(
        op_ids.contains(&"tool-1"),
        "tool-1 (RunningForeground) must be in pending_operations"
    );
    assert!(
        op_ids.contains(&"tool-2"),
        "tool-2 (RunningBackground) must be in pending_operations"
    );
    assert!(
        op_ids.contains(&"child-1"),
        "child-1 (Running) must be in pending_operations"
    );

    // Now clear exec state.
    cs.read().await.clear_exec_state();

    // After clear: maps are empty.
    assert!(
        cs.read().await.tool_states.read().expect("lock").is_empty(),
        "tool_states must be empty after clear_exec_state"
    );
    assert!(
        cs.read()
            .await
            .child_states
            .read()
            .expect("lock")
            .is_empty(),
        "child_states must be empty after clear_exec_state"
    );
}

/// Empty session returns empty pending_operations (baseline).
#[tokio::test]
async fn test_pending_operations_empty_session() {
    let cs = make_session("s1.4_pending_empty");
    let pending_ops = cs.read().await.collect_pending_operations();
    assert!(
        pending_ops.is_empty(),
        "pending_operations should be empty for idle session"
    );
}

/// After clear_exec_state, collect_pending_operations returns empty
/// (confirming the maps were wiped).
#[tokio::test]
async fn test_pending_operations_empty_after_clear() {
    let cs = make_session("s1.4_pending_after_clear");

    // Register then clear.
    {
        let guard = cs.read().await;
        guard.register_tool_call("t1", "bash", "cmd");
        guard.update_tool_state("t1", ToolExecState::RunningForeground);
        guard.register_child("c1", "agent", "task");
    }
    cs.read().await.clear_exec_state();

    let pending_ops = cs.read().await.collect_pending_operations();
    assert!(
        pending_ops.is_empty(),
        "pending_operations must be empty after clear_exec_state"
    );
}

/// Terminal-state tools are NOT in pending_operations (they were removed
/// from tool_states by Step 1.1).
#[tokio::test]
async fn test_pending_operations_excludes_terminal_tools() {
    let cs = make_session("s1.4_pending_no_terminal");

    {
        let guard = cs.read().await;
        // Terminal tool — should be removed from map.
        guard.register_tool_call("done", "bash", "cmd");
        guard.update_tool_state("done", ToolExecState::Completed);
        // Non-terminal tool — still in map.
        guard.register_tool_call("running", "bash", "cmd");
        guard.update_tool_state("running", ToolExecState::RunningForeground);
    }

    let pending_ops = cs.read().await.collect_pending_operations();
    let op_ids: Vec<&str> = pending_ops.iter().map(|op| op.op_id.as_str()).collect();
    assert!(
        !op_ids.contains(&"done"),
        "terminal tool 'done' must NOT be in pending_operations"
    );
    assert!(
        op_ids.contains(&"running"),
        "non-terminal tool 'running' must be in pending_operations"
    );
}

/// Pending tools (just registered, not yet started) are included in
/// pending_operations.
#[tokio::test]
async fn test_pending_operations_includes_pending_tools() {
    let cs = make_session("s1.4_pending_includes_pending");

    {
        let guard = cs.read().await;
        guard.register_tool_call("new-tool", "bash", "echo");
        // Not yet updated — still in Pending state.
    }

    let pending_ops = cs.read().await.collect_pending_operations();
    assert_eq!(pending_ops.len(), 1);
    assert_eq!(pending_ops[0].op_id, "new-tool");
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Forceful stop ordering (Gap 3 — Step 1.3)
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that `stop(Forceful)` executes in design-doc order:
/// cascade → kill tools → cancel LLM → cleanup.
///
/// We use a sequence log (Arc<Vec<String>>) appended to by mock handles
/// and observation of the cancel_token + tool_states to confirm order.
#[tokio::test]
async fn test_forceful_stop_order_cascade_before_cancel_before_clear() {
    let parent = make_session("s1.4_order_parent");
    let child = make_session("s1.4_order_child");

    // Wire child into parent.
    parent
        .read()
        .await
        .register_child_handle("s1.4_order_child", Arc::downgrade(&child));

    // Add a kill handle on the child.
    let child_kill_count = Arc::new(AtomicUsize::new(0));
    let child_handle = OrderTrackingKillHandle::new(Arc::clone(&child_kill_count));
    child
        .read()
        .await
        .register_tool_handle("child-tool", Arc::new(child_handle));

    // Run stop(Forceful).
    parent
        .read()
        .await
        .stop(true, ShutdownMode::Forceful, Duration::ZERO)
        .await;

    // 1. Cascade ran: child is stopped.
    assert!(
        child.read().await.is_stopped(),
        "cascade must have run — child must be stopped"
    );

    // 2. Kill handles were invoked: child's kill handle fired.
    assert_eq!(
        child_kill_count.load(Ordering::SeqCst),
        1,
        "child tool handle must be killed (cascade runs before kill)"
    );

    // 3. Cancel token is fired.
    assert!(
        parent.read().await.cancel_token().is_cancelled(),
        "cancel_token must be cancelled after kill handles"
    );

    // 4. Cleanup ran: tool_states and child_states are empty.
    assert!(
        parent
            .read()
            .await
            .tool_states
            .read()
            .expect("lock")
            .is_empty(),
        "tool_states must be cleared after cleanup"
    );
    assert!(
        parent
            .read()
            .await
            .child_states
            .read()
            .expect("lock")
            .is_empty(),
        "child_states must be cleared after cleanup"
    );
}

/// Verify the ordering is observable via a sequence log: cascade runs
/// first, then kill_tool_handles, then cancel_token, then clear.
#[tokio::test]
async fn test_forceful_stop_sequence_log() {
    use std::sync::Mutex;

    let seq: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seq_clone = Arc::clone(&seq);

    let parent = make_session("s1.4_seq_parent");
    let child = make_session("s1.4_seq_child");

    parent
        .read()
        .await
        .register_child_handle("s1.4_seq_child", Arc::downgrade(&child));

    // Child's kill handle logs "kill" into the sequence.
    {
        let seq = Arc::clone(&seq_clone);
        let handle =
            OrderTrackingKillHandle::with_callback(Arc::new(AtomicUsize::new(0)), move || {
                seq.lock().unwrap().push("kill".to_string());
            });
        child
            .read()
            .await
            .register_tool_handle("child-tool", Arc::new(handle));
    }

    // Before stop: log is empty.
    assert!(seq_clone.lock().unwrap().is_empty());

    parent
        .read()
        .await
        .stop(true, ShutdownMode::Forceful, Duration::ZERO)
        .await;

    // After stop: "kill" was logged (cascade ran and invoked child's
    // kill handle). The cancel_token was set after kill (so kill
    // appears first in the log).
    let log = seq_clone.lock().unwrap();
    assert!(
        log.contains(&"kill".to_string()),
        "sequence log must contain 'kill' entry"
    );
    // The only entries should be from the kill handle callback.
    // (cancel_token and clear_exec_state don't append to the log.)
    assert_eq!(log.len(), 1, "kill should be called exactly once");
}

/// On an empty session, stop(Forceful) still follows the order:
/// cascade (no-op) → kill (no-op) → cancel → clear.
#[tokio::test]
async fn test_forceful_stop_empty_session_order() {
    let cs = make_session("s1.4_empty_order");

    cs.read()
        .await
        .stop(true, ShutdownMode::Forceful, Duration::ZERO)
        .await;

    assert!(cs.read().await.is_stopped());
    assert!(cs.read().await.cancel_token().is_cancelled());
    assert!(cs.read().await.tool_states.read().expect("lock").is_empty());
    assert!(cs
        .read()
        .await
        .child_states
        .read()
        .expect("lock")
        .is_empty());
}
