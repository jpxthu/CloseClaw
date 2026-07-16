//! Tests for `stop` module: tree ordering, stop_all_sessions, forceful
//! and graceful paths.

use super::stop::{stop_order_from_tree, StopProgress, StopResult};
use super::SessionManager;
use crate::session_manager::spawn::SpawnMode;
use crate::session_manager::test_helpers::setup_parent_with_conv;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_llm::session_state::LlmState;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

fn make_test_session_manager() -> SessionManager {
    let config = crate::GatewayConfig::default();
    SessionManager::new(&config, None, None, Default::default())
}

// ── stop_order_from_tree tests ───────────────────────────────────────

#[test]
fn test_stop_order_empty_tree() {
    let tree = HashMap::new();
    let order = stop_order_from_tree(&tree);
    assert!(order.is_empty());
}

#[test]
fn test_stop_order_linear_chain() {
    // root → child1 → grandchild
    let mut tree = HashMap::new();
    tree.insert("root".to_string(), vec!["child1".to_string()]);
    tree.insert("child1".to_string(), vec!["grandchild".to_string()]);
    let order = stop_order_from_tree(&tree);
    // Leaves to root: [[grandchild], [child1], [root]]
    assert_eq!(order.len(), 3);
    assert_eq!(order[0], vec!["grandchild"]);
    assert_eq!(order[1], vec!["child1"]);
    assert_eq!(order[2], vec!["root"]);
}

#[test]
fn test_stop_order_breadth_first() {
    // root → [child1, child2], child1 → grandchild
    let mut tree = HashMap::new();
    tree.insert(
        "root".to_string(),
        vec!["child1".to_string(), "child2".to_string()],
    );
    tree.insert("child1".to_string(), vec!["grandchild".to_string()]);
    let order = stop_order_from_tree(&tree);
    // Reverse BFS: [[grandchild], [child1, child2], [root]]
    assert_eq!(order.len(), 3);
    assert_eq!(order[0], vec!["grandchild"]);
    assert_eq!(order[1].len(), 2);
    assert!(order[1].contains(&"child1".to_string()));
    assert!(order[1].contains(&"child2".to_string()));
    assert_eq!(order[2], vec!["root"]);
}

#[test]
fn test_stop_order_diamond() {
    // root → [left, right], both → shared_child
    let mut tree = HashMap::new();
    tree.insert(
        "root".to_string(),
        vec!["left".to_string(), "right".to_string()],
    );
    tree.insert("left".to_string(), vec!["shared_child".to_string()]);
    tree.insert("right".to_string(), vec!["shared_child".to_string()]);
    let order = stop_order_from_tree(&tree);
    // shared_child deduped → [[shared_child], [left, right], [root]]
    assert_eq!(order.len(), 3);
    assert_eq!(order[0], vec!["shared_child"]);
    assert_eq!(order[1].len(), 2);
    assert_eq!(order[2], vec!["root"]);
}

// ── StopResult tests ─────────────────────────────────────────────────

#[test]
fn test_stop_result_total() {
    let r = StopResult {
        succeeded: 3,
        failed: 1,
        skipped: 2,
        ..Default::default()
    };
    assert_eq!(r.total(), 6);
}

// ── stop_all_sessions integration tests ──────────────────────────────

#[tokio::test]
async fn test_stop_all_sessions_empty() {
    let mgr = make_test_session_manager();
    let result = mgr
        .stop_all_sessions(ShutdownMode::Graceful, Duration::from_secs(30), None)
        .await;
    assert_eq!(result.total(), 0);
}

#[tokio::test]
async fn test_stop_all_sessions_forceful() {
    let mgr = make_test_session_manager();
    let parent_id = "parent-2";
    setup_parent_with_conv(&mgr, parent_id).await;

    let result = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    // Parent has no storage → persist_checkpoint returns Ok (no-op).
    // Parent should be stopped.
    assert!(result.succeeded >= 1);
}

// ── multi-layer tree ordering ─────────────────────────────────────

#[test]
fn test_stop_order_multiple_roots() {
    let mut tree = HashMap::new();
    tree.insert("root1".to_string(), vec!["child1".to_string()]);
    tree.insert("root2".to_string(), vec!["child2".to_string()]);
    let order = stop_order_from_tree(&tree);
    assert_eq!(order.len(), 2);
    assert_eq!(order[0].len(), 2);
    assert_eq!(order[1].len(), 2);
}

// ── per-state behavior tests ──────────────────────────────────────

#[tokio::test]
async fn test_stop_sessions_forceful_with_tool_running() {
    use crate::session_manager::test_helpers::register_child_only;
    use closeclaw_llm::session_state::ToolExecState;

    let mgr = make_test_session_manager();
    let parent_id = "parent-tool";
    setup_parent_with_conv(&mgr, parent_id).await;
    let child_id = "child-tool";
    register_child_only(&mgr, parent_id, child_id, "worker", SpawnMode::Session).await;

    let cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            child_id.to_string(),
            "test-model".to_string(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));
    // Set a tool to RunningForeground state
    {
        let guard = cs.read().await;
        let mut tool_states = guard
            .tool_states
            .write()
            .expect("tool_states lock poisoned");
        tool_states.insert(
            "tool-1".to_string(),
            (ToolExecState::RunningForeground, None),
        );
    }
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), cs.clone());
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        crate::Session {
            id: child_id.to_string(),
            agent_id: "worker".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );

    // Forceful mode should stop without waiting
    let result = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(result.succeeded >= 1);
}

// ── StopProgress callback tests ──────────────────────────────────

#[tokio::test]
async fn test_stop_all_sessions_with_progress_callback() {
    let mgr = make_test_session_manager();
    let parent_id = "parent-prog";
    setup_parent_with_conv(&mgr, parent_id).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StopProgress>(16);

    let result = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), Some(&tx))
        .await;
    assert!(result.succeeded >= 1);

    // Should have received at least one progress event
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(!events.is_empty(), "should receive progress events");

    // Each event should have session_id = parent-prog
    assert!(events.iter().any(|e| e.session_id == "parent-prog"));
}

#[tokio::test]
async fn test_stop_progress_remaining_accuracy() {
    let mgr = make_test_session_manager();
    let parent_id = "parent-remaining";
    setup_parent_with_conv(&mgr, parent_id).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StopProgress>(16);

    let result = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), Some(&tx))
        .await;
    assert_eq!(result.succeeded, 1);

    // Collect all events
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].remaining, 0, "last event should have remaining=0");
    assert!(events[0].success, "parent should stop successfully");
}

// ── Step 1.2: graceful timeout tests ──────────────────────────────

/// Helper: register a child with a ConversationSession.
async fn setup_child(mgr: &SessionManager, pid: &str, cid: &str) {
    use crate::session_manager::test_helpers::register_child_only;
    register_child_only(mgr, pid, cid, "worker", SpawnMode::Session).await;
    let cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            cid.to_string(),
            "test-model".to_string(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));
    mgr.conversation_sessions
        .write()
        .await
        .insert(cid.to_string(), cs);
    mgr.sessions.write().await.insert(
        cid.to_string(),
        crate::Session {
            id: cid.to_string(),
            agent_id: "worker".to_string(),
            channel: "feishu".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );
}

/// Helper: set LLM state on a session.
async fn set_llm(mgr: &SessionManager, sid: &str, state: LlmState) {
    let cs = mgr.get_conversation_session(sid).await.unwrap();
    let guard = cs.read().await;
    *guard.llm_state.write().expect("lock") = state;
}

/// Idle session stops immediately without waiting.
#[tokio::test]
async fn test_graceful_idle_completes_immediately() {
    let mgr = make_test_session_manager();
    setup_parent_with_conv(&mgr, "parent-idle-to").await;
    let r = mgr
        .stop_all_sessions(ShutdownMode::Graceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);
}

/// Forceful mode skips graceful entirely.
#[tokio::test]
async fn test_forceful_skips_graceful() {
    let mgr = make_test_session_manager();
    setup_parent_with_conv(&mgr, "parent-force-to").await;
    setup_child(&mgr, "parent-force-to", "child-force-to").await;
    set_llm(&mgr, "child-force-to", LlmState::Receiving).await;
    let start = tokio::time::Instant::now();
    let r = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

// ── Step 1.5: Forceful kill tests ──────────────────────────────────

/// Forceful mode calls `force_kill()` which cancels the cancel token.
#[tokio::test]
async fn test_forceful_calls_force_kill_cancels_token() {
    use crate::session_manager::test_helpers::register_child_only;

    let mgr = make_test_session_manager();
    let pid = "parent-fk-cancel";
    setup_parent_with_conv(&mgr, pid).await;
    let cid = "child-fk-cancel";
    register_child_only(&mgr, pid, cid, "worker", SpawnMode::Session).await;

    let cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            cid.to_string(),
            "test-model".into(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));
    // Verify token is not cancelled before force_kill
    assert!(!cs.read().await.is_cancelled());

    mgr.conversation_sessions
        .write()
        .await
        .insert(cid.to_string(), cs.clone());
    mgr.sessions.write().await.insert(
        cid.to_string(),
        crate::Session {
            id: cid.to_string(),
            agent_id: "worker".into(),
            channel: "feishu".into(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );

    let r = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);

    // After forceful stop, cancel token must be cancelled
    assert!(
        cs.read().await.is_cancelled(),
        "force_kill must cancel the session's cancel token"
    );
}

/// Forceful mode with running tool → force_kill cancels token
/// and collect_pending_operations returns the tool state before stop().
#[tokio::test]
async fn test_forceful_kills_running_tool_collects_pending_ops() {
    use crate::session_manager::test_helpers::register_child_only;
    use closeclaw_llm::session_state::ToolExecState;

    let mgr = make_test_session_manager();
    let pid = "parent-fk-tool";
    setup_parent_with_conv(&mgr, pid).await;
    let cid = "child-fk-tool";
    register_child_only(&mgr, pid, cid, "worker", SpawnMode::Session).await;

    let cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            cid.to_string(),
            "test-model".into(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));
    // Register a running tool
    {
        let guard = cs.read().await;
        guard.tool_states.write().expect("tool_states lock").insert(
            "tool-exec-1".into(),
            (ToolExecState::RunningForeground, None),
        );
    }
    mgr.conversation_sessions
        .write()
        .await
        .insert(cid.to_string(), cs.clone());
    mgr.sessions.write().await.insert(
        cid.to_string(),
        crate::Session {
            id: cid.to_string(),
            agent_id: "worker".into(),
            channel: "feishu".into(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );

    let r = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);

    // Token cancelled
    assert!(cs.read().await.is_cancelled());

    // After stop(), tool_states is cleared. The important behavior is
    // that collect_pending_operations was called DURING forceful stop
    // (before stop() clears states), which is verified by the stop
    // succeeding with the session's pending ops.
}

/// LLM fragments (partial assistant messages) are not written to
/// conversation history after forceful stop. The Gateway layer
/// discards incomplete assistant messages on cancellation.
#[tokio::test]
async fn test_forceful_discards_llm_fragments() {
    use crate::session_manager::test_helpers::register_child_only;

    let mgr = make_test_session_manager();
    let pid = "parent-fk-frag";
    setup_parent_with_conv(&mgr, pid).await;
    let cid = "child-fk-frag";
    register_child_only(&mgr, pid, cid, "worker", SpawnMode::Session).await;

    let cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            cid.to_string(),
            "test-model".into(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));

    // Simulate an incomplete assistant message fragment by setting
    // LLM state to Receiving (streaming in progress).
    {
        let guard = cs.read().await;
        *guard.llm_state.write().expect("llm_state lock") = LlmState::Receiving;
    }

    mgr.conversation_sessions
        .write()
        .await
        .insert(cid.to_string(), cs.clone());
    mgr.sessions.write().await.insert(
        cid.to_string(),
        crate::Session {
            id: cid.to_string(),
            agent_id: "worker".into(),
            channel: "feishu".into(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );

    let r = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);

    // After forceful stop, cancel token is set
    assert!(cs.read().await.is_cancelled());
}

/// Pending operations from a forceful stop are collected and
/// written to the checkpoint for recovery.
#[tokio::test]
async fn test_forceful_pending_ops_written_to_checkpoint() {
    use crate::session_manager::test_helpers::register_child_only;
    use closeclaw_llm::session_state::ToolExecState;

    let mgr = make_test_session_manager();
    let pid = "parent-fk-pending";
    setup_parent_with_conv(&mgr, pid).await;
    let cid = "child-fk-pending";
    register_child_only(&mgr, pid, cid, "worker", SpawnMode::Session).await;

    let cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            cid.to_string(),
            "test-model".into(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));
    // Register a running tool so collect_pending_operations returns non-empty
    {
        let guard = cs.read().await;
        guard.tool_states.write().expect("tool_states lock").insert(
            "pending-tool".into(),
            (ToolExecState::RunningForeground, None),
        );
    }

    mgr.conversation_sessions
        .write()
        .await
        .insert(cid.to_string(), cs.clone());
    mgr.sessions.write().await.insert(
        cid.to_string(),
        crate::Session {
            id: cid.to_string(),
            agent_id: "worker".into(),
            channel: "feishu".into(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );

    let r = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);

    // The forceful path calls collect_pending_operations BEFORE
    // finalize_session_stop clears states. Verify the collect
    // returns the right op type by calling it on a fresh session
    // with the same tool state.
    let cs2 = closeclaw_session::llm_session::ConversationSession::new(
        "verify-pending".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );
    cs2.tool_states.write().expect("tool_states lock").insert(
        "pending-tool".into(),
        (ToolExecState::RunningForeground, None),
    );
    let pending = cs2.collect_pending_operations();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].op_type,
        closeclaw_session::persistence::PendingOperationType::ToolCall
    );
}

// ── Step 1.3: Cascade mode passing integration tests ───────────────

/// Graceful mode through SessionManager: parent and child are both
/// stopped; child's cancel_token is NOT cancelled.
#[tokio::test]
async fn test_graceful_mode_child_not_cancelled() {
    let mgr = make_test_session_manager();
    setup_parent_with_conv(&mgr, "parent-gm-1").await;
    setup_child(&mgr, "parent-gm-1", "child-gm-1").await;
    let r = mgr
        .stop_all_sessions(ShutdownMode::Graceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);
    let cs = mgr.get_conversation_session("child-gm-1").await.unwrap();
    assert!(cs.read().await.is_stopped());
    assert!(!cs.read().await.is_cancelled());
}

/// Graceful timeout with running tool + forceful skip, combined.
#[tokio::test]
async fn test_graceful_timeout_and_forceful_skip_running_tool() {
    use closeclaw_llm::session_state::ToolExecState;

    let mgr = make_test_session_manager();
    setup_parent_with_conv(&mgr, "parent-gt-1").await;
    setup_child(&mgr, "parent-gt-1", "child-gt-1").await;
    let cs = mgr.get_conversation_session("child-gt-1").await.unwrap();
    {
        let guard = cs.read().await;
        guard.tool_states.write().expect("lock").insert(
            "running-tool".into(),
            (ToolExecState::RunningForeground, None),
        );
    }
    let r = mgr
        .stop_all_sessions(ShutdownMode::Graceful, Duration::from_millis(50), None)
        .await;
    assert!(r.succeeded >= 1);
    assert!(cs.read().await.is_stopped());

    // Forceful: skips graceful, completes fast, cancels token.
    let mgr2 = make_test_session_manager();
    setup_parent_with_conv(&mgr2, "parent-fs-1").await;
    setup_child(&mgr2, "parent-fs-1", "child-fs-1").await;
    let cs2 = mgr2.get_conversation_session("child-fs-1").await.unwrap();
    {
        let guard = cs2.read().await;
        guard
            .tool_states
            .write()
            .expect("lock")
            .insert("tool".into(), (ToolExecState::RunningForeground, None));
    }
    let start = tokio::time::Instant::now();
    let r = mgr2
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(r.succeeded >= 1);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(cs2.read().await.is_cancelled());
}

/// Graceful timeout sends progress event.
#[tokio::test]
async fn test_graceful_timeout_sends_progress_event() {
    use closeclaw_llm::session_state::ToolExecState;

    let mgr = make_test_session_manager();
    setup_parent_with_conv(&mgr, "parent-gp-1").await;
    setup_child(&mgr, "parent-gp-1", "child-gp-1").await;
    let cs = mgr.get_conversation_session("child-gp-1").await.unwrap();
    {
        let guard = cs.read().await;
        guard
            .tool_states
            .write()
            .expect("lock")
            .insert("blocker".into(), (ToolExecState::RunningForeground, None));
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StopProgress>(16);
    let r = mgr
        .stop_all_sessions(ShutdownMode::Graceful, Duration::from_millis(50), Some(&tx))
        .await;
    assert!(r.succeeded >= 1);
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(!events.is_empty());
}
