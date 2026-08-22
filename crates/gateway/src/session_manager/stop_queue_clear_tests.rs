//! Tests for `/stop` queue clearing (must_fix 1, Step 1.1).
//!
//! Covers:
//! - `/stop` clears unified message queue (mixed priority)
//! - After clearing, checkpoint pending messages do not contain discarded queue entries
//! - System shutdown path does NOT clear queue
//! - `clear_queue()` returns correct count

use super::spawn::SpawnMode;
use super::stop::StopOptions;
use super::test_helpers::{register_child_only, setup_parent_with_conv};
use super::tests::make_test_mgr;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_session::persistence::PendingMessage;
use closeclaw_tasks::NotificationPriority;
use std::sync::Arc;
use std::time::Duration;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Push multiple entries of different types onto a session's unified queue.
async fn push_mixed_entries(mgr: &super::SessionManager, session_id: &str) {
    use chrono::Utc;
    use closeclaw_session::llm_session::AnnounceEvent;

    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let mut cs = cs.write().await;

    // User message (Later priority)
    cs.push_pending(PendingMessage::new("user-1".into(), "hello".into()));

    // Announce event with Now priority
    cs.push_announce_to_queue(AnnounceEvent {
        child_session_id: "child-now".into(),
        child_agent_id: "agent-now".into(),
        result_text: "urgent result".into(),
        completed_at: Utc::now(),
        priority: NotificationPriority::Now,
        status: closeclaw_common::ChildCompletionStatus::Completed,
    });

    // Announce event with Next priority
    cs.push_announce_to_queue(AnnounceEvent {
        child_session_id: "child-next".into(),
        child_agent_id: "agent-next".into(),
        result_text: "next result".into(),
        completed_at: Utc::now(),
        priority: NotificationPriority::Next,
        status: closeclaw_common::ChildCompletionStatus::Completed,
    });

    // System notification with Later priority
    cs.push_system_notification("system msg".into(), NotificationPriority::Later);

    // Another user message
    cs.push_pending(PendingMessage::new("user-2".into(), "world".into()));
}

/// Get the queue length for a session.
async fn queue_len(mgr: &super::SessionManager, session_id: &str) -> usize {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let guard = cs.read().await;
    guard.pending_count()
}

/// Get the number of pending user messages for a session (from queue).
async fn pending_msg_count(mgr: &super::SessionManager, session_id: &str) -> usize {
    let cs = mgr.get_conversation_session(session_id).await.unwrap();
    let guard = cs.read().await;
    guard.get_pending_messages().len()
}

// ── /stop clears queue ─────────────────────────────────────────────────────

/// `/stop` (via `gw_stop`) calls `stop_single_session` with
/// `clear_queue=true`. After Forceful stop, the unified queue must
/// be empty.
#[tokio::test]
async fn test_stop_clears_queue_forceful() {
    let mgr = make_test_mgr(None);
    let pid = "parent-stop-clear";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;
    assert!(queue_len(&mgr, pid).await >= 4, "queue should have entries");

    // gw_stop passes clear_queue=true → Forceful path clears queue
    let result = mgr
        .stop_single_session(
            pid,
            ShutdownMode::Forceful,
            false,
            StopOptions {
                timeout: Duration::from_secs(30),
                progress_tx: None,
                clear_queue: true,
            },
        )
        .await;
    assert!(result.is_ok(), "stop should succeed");

    assert_eq!(
        queue_len(&mgr, pid).await,
        0,
        "queue must be empty after /stop"
    );
}

/// `/stop` with Graceful mode also clears the queue when `clear_queue=true`.
#[tokio::test]
async fn test_stop_clears_queue_graceful() {
    let mgr = make_test_mgr(None);
    let pid = "parent-stop-grace";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;
    assert!(queue_len(&mgr, pid).await >= 4);

    let result = mgr
        .stop_single_session(
            pid,
            ShutdownMode::Graceful,
            false,
            StopOptions {
                timeout: Duration::from_secs(30),
                progress_tx: None,
                clear_queue: true,
            },
        )
        .await;
    assert!(result.is_ok());

    assert_eq!(
        queue_len(&mgr, pid).await,
        0,
        "queue must be empty after graceful /stop"
    );
}

/// `/stop` clears queue: mixed priority queue with all three levels.
#[tokio::test]
async fn test_stop_clears_mixed_priority_queue() {
    let mgr = make_test_mgr(None);
    let pid = "parent-stop-mix";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;
    let before = queue_len(&mgr, pid).await;
    assert!(before >= 4, "expected at least 4 entries, got {}", before);

    let result = mgr
        .stop_single_session(
            pid,
            ShutdownMode::Forceful,
            false,
            StopOptions {
                timeout: Duration::from_secs(30),
                progress_tx: None,
                clear_queue: true,
            },
        )
        .await;
    assert!(result.is_ok());
    assert_eq!(queue_len(&mgr, pid).await, 0);
}

// ── checkpoint pending excludes cleared queue entries ───────────────────────

/// After `/stop` clears the queue, `pending_messages_for` must return
/// an empty list — the cleared entries must NOT appear in the
/// checkpoint's pending messages.
#[tokio::test]
async fn test_stop_checkpoint_pending_excludes_cleared_queue() {
    let mgr = make_test_mgr(None);
    let pid = "parent-stop-cp";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;

    // Verify queue has user messages before stop
    assert!(
        pending_msg_count(&mgr, pid).await >= 2,
        "should have user messages"
    );

    let result = mgr
        .stop_single_session(
            pid,
            ShutdownMode::Forceful,
            false,
            StopOptions {
                timeout: Duration::from_secs(30),
                progress_tx: None,
                clear_queue: true,
            },
        )
        .await;
    assert!(result.is_ok());

    // After clear_queue + persist, pending_messages_for should return empty
    let pending = {
        let conv = mgr.conversation_sessions.read().await;
        match conv.get(pid) {
            Some(cs) => {
                let guard = cs.read().await;
                guard.get_pending_messages()
            }
            None => Vec::new(),
        }
    };
    assert!(
        pending.is_empty(),
        "checkpoint pending messages must be empty after /stop queue clear, got: {:?}",
        pending
    );
}

/// After `/stop` clears the queue, `get_announce_events` must also
/// return empty — announce events were also discarded.
#[tokio::test]
async fn test_stop_checkpoint_announce_events_excluded() {
    use chrono::Utc;
    use closeclaw_session::llm_session::AnnounceEvent;

    let mgr = make_test_mgr(None);
    let pid = "parent-stop-ann-cp";
    setup_parent_with_conv(&mgr, pid).await;

    // Push announce events only
    {
        let cs = mgr.get_conversation_session(pid).await.unwrap();
        let mut cs = cs.write().await;
        cs.push_announce_to_queue(AnnounceEvent {
            child_session_id: "child-a".into(),
            child_agent_id: "agent-a".into(),
            result_text: "result a".into(),
            completed_at: Utc::now(),
            priority: NotificationPriority::Now,
            status: closeclaw_common::ChildCompletionStatus::Completed,
        });
        cs.push_announce_to_queue(AnnounceEvent {
            child_session_id: "child-b".into(),
            child_agent_id: "agent-b".into(),
            result_text: "result b".into(),
            completed_at: Utc::now(),
            priority: NotificationPriority::Next,
            status: closeclaw_common::ChildCompletionStatus::Completed,
        });
    }

    assert_eq!(queue_len(&mgr, pid).await, 2);

    let result = mgr
        .stop_single_session(
            pid,
            ShutdownMode::Forceful,
            false,
            StopOptions {
                timeout: Duration::from_secs(30),
                progress_tx: None,
                clear_queue: true,
            },
        )
        .await;
    assert!(result.is_ok());

    // Announce events must be gone
    let announces = {
        let conv = mgr.conversation_sessions.read().await;
        match conv.get(pid) {
            Some(cs) => {
                let guard = cs.read().await;
                guard.get_announce_events()
            }
            None => Vec::new(),
        }
    };
    assert!(
        announces.is_empty(),
        "announce events must be empty after /stop queue clear, got: {:?}",
        announces
    );
}

// ── System shutdown path does NOT clear queue ──────────────────────────────

/// `stop_all_sessions` (system shutdown) calls `stop_single_session`
/// with `clear_queue=false`. The queue must remain intact after stop.
#[tokio::test]
async fn test_system_shutdown_preserves_queue() {
    let mgr = make_test_mgr(None);
    let pid = "parent-sys-shutdown";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;
    let before = queue_len(&mgr, pid).await;
    assert!(before >= 4);

    // stop_all_sessions → process_stop_level → stop_single_session(clear_queue=false)
    let result = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(result.succeeded >= 1);

    // Queue should still have the original entries
    assert_eq!(
        queue_len(&mgr, pid).await,
        before,
        "queue must be preserved after system shutdown"
    );
}

/// System shutdown with Graceful mode also preserves queue.
#[tokio::test]
async fn test_system_shutdown_graceful_preserves_queue() {
    let mgr = make_test_mgr(None);
    let pid = "parent-sys-grace";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;
    let before = queue_len(&mgr, pid).await;

    let result = mgr
        .stop_all_sessions(ShutdownMode::Graceful, Duration::from_secs(30), None)
        .await;
    assert!(result.succeeded >= 1);

    assert_eq!(
        queue_len(&mgr, pid).await,
        before,
        "queue must be preserved after graceful system shutdown"
    );
}

/// System shutdown preserves pending user messages in checkpoint.
#[tokio::test]
async fn test_system_shutdown_preserves_pending_in_checkpoint() {
    let mgr = make_test_mgr(None);
    let pid = "parent-sys-cp";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;

    let result = mgr
        .stop_all_sessions(ShutdownMode::Forceful, Duration::from_secs(30), None)
        .await;
    assert!(result.succeeded >= 1);

    // pending_messages_for should still return the user messages
    let pending = {
        let conv = mgr.conversation_sessions.read().await;
        match conv.get(pid) {
            Some(cs) => {
                let guard = cs.read().await;
                guard.get_pending_messages()
            }
            None => Vec::new(),
        }
    };
    assert!(
        !pending.is_empty(),
        "pending messages must be preserved after system shutdown"
    );
}

// ── clear_queue returns correct count ──────────────────────────────────────

/// `clear_queue()` returns the number of entries removed.
#[tokio::test]
async fn test_clear_queue_returns_correct_count() {
    let mgr = make_test_mgr(None);
    let pid = "parent-clear-count";
    setup_parent_with_conv(&mgr, pid).await;

    push_mixed_entries(&mgr, pid).await;

    let cs = mgr.get_conversation_session(pid).await.unwrap();
    let count = { cs.write().await.clear_queue() };
    assert!(count >= 4, "clear_queue should return >= 4, got {}", count);
    assert_eq!(cs.read().await.pending_count(), 0);
}

/// `clear_queue()` on empty queue returns 0.
#[tokio::test]
async fn test_clear_queue_empty_returns_zero() {
    let mgr = make_test_mgr(None);
    let pid = "parent-clear-empty";
    setup_parent_with_conv(&mgr, pid).await;

    let cs = mgr.get_conversation_session(pid).await.unwrap();
    let count = { cs.write().await.clear_queue() };
    assert_eq!(count, 0);
}

// ── /stop clear_queue on child session ─────────────────────────────────────

/// `/stop` on a parent with children: parent queue cleared, child
/// queue not directly affected (child is stopped separately).
#[tokio::test]
async fn test_stop_clears_parent_queue_not_child() {
    let mgr = make_test_mgr(None);
    let pid = "parent-stop-child";
    setup_parent_with_conv(&mgr, pid).await;
    let cid = "child-stop-clear";
    register_child_only(&mgr, pid, cid, "worker", SpawnMode::Session).await;

    let child_cs = Arc::new(tokio::sync::RwLock::new(
        closeclaw_session::llm_session::ConversationSession::new(
            cid.to_string(),
            "test-model".into(),
            std::path::PathBuf::from("/tmp"),
        ),
    ));
    // Push entries to child queue
    {
        let mut cs = child_cs.write().await;
        cs.push_pending(PendingMessage::new("child-msg".into(), "from child".into()));
    }
    mgr.conversation_sessions
        .write()
        .await
        .insert(cid.to_string(), child_cs.clone());
    mgr.sessions.write().await.insert(
        cid.to_string(),
        super::Session {
            id: cid.to_string(),
            agent_id: "worker".into(),
            channel: "feishu".into(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 1,
        },
    );

    // Push entries to parent queue
    push_mixed_entries(&mgr, pid).await;

    // /stop parent with clear_queue=true
    let result = mgr
        .stop_single_session(
            pid,
            ShutdownMode::Forceful,
            false,
            StopOptions {
                timeout: Duration::from_secs(30),
                progress_tx: None,
                clear_queue: true,
            },
        )
        .await;
    assert!(result.is_ok());

    // Parent queue must be empty
    assert_eq!(queue_len(&mgr, pid).await, 0);

    // Child queue must still have its entry (child stop is separate)
    let child_pending = {
        let guard = child_cs.read().await;
        guard.get_pending_messages()
    };
    assert_eq!(
        child_pending.len(),
        1,
        "child queue should still have its pending message"
    );
}
