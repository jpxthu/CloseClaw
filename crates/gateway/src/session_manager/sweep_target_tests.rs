//! Tests for `AnnounceSweepTarget` trait implementation on
//! `SessionManager` (stale-child detection path).
//!
//! Covers:
//! - `get_last_output_at`: session exists / doesn't exist
//! - `is_parent_archived`: active / archived / missing
//! - `terminate_stale_child`: kill + notification when parent active
//! - `terminate_stale_child`: kill + no notification when parent archived
//! - `terminate_stale_child`: kill still fires when kill_child fails
//! - Cascade: nested descendants terminated on stale kill

use super::spawn::SpawnMode;
use super::test_helpers::{append_assistant_to_child, setup_parent_with_conv};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_common::{tool_session::ToolSession, ToolExecState};
use closeclaw_session::run_health::AnnounceSweepTarget;
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;

fn make_msg() -> crate::Message {
    use std::collections::HashMap;
    crate::Message {
        id: "msg_sweep".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hello".into(),
        channel: "ch".into(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

// ── 1. get_last_output_at: session exists → returns timestamp ────────────

/// `get_last_output_at` returns `Some(ts)` for a session that has an
/// assistant message appended (which refreshes `last_activity_at`).
#[tokio::test]
#[serial]
async fn test_get_last_output_at_exists() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-glo").await;

    let child_id = mgr
        .create_child_session(
            &super::test_helpers::test_resolved_config("worker-glo", None),
            &parent_id,
            1,
            "work",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .expect("create_child_session");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text("done".into())],
    )
    .await;

    let ts = mgr.get_last_output_at(&child_id).await;
    assert!(ts.is_some(), "should return a timestamp for known session");
    // Timestamp should be a reasonable recent value.
    let ts = ts.unwrap();
    assert!(ts > 0, "timestamp should be positive, got {}", ts);
}

// ── 2. get_last_output_at: unknown session → None ────────────────────────

/// `get_last_output_at` returns `None` for a session id that has no
/// `ConversationSession` registered.
#[tokio::test]
#[serial]
async fn test_get_last_output_at_unknown() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let result = mgr.get_last_output_at("nonexistent-session").await;
    assert!(result.is_none(), "unknown session should return None");
}

// ── 3. is_parent_archived: active parent → false ─────────────────────────

/// `is_parent_archived` returns `false` when the parent session is
/// registered in the active sessions map.
#[tokio::test]
#[serial]
async fn test_is_parent_archived_active() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let _parent_id = setup_parent_with_conv(&mgr, "parent-active").await;

    assert!(
        !mgr.is_parent_archived("parent-active").await,
        "active parent should not be archived"
    );
}

// ── 4. is_parent_archived: missing parent → true ─────────────────────────

/// `is_parent_archived` returns `true` when the parent session id is
/// not in the sessions map (simulates archived/removed).
#[tokio::test]
#[serial]
async fn test_is_parent_archived_missing() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    assert!(
        mgr.is_parent_archived("nonexistent-parent").await,
        "missing parent should be treated as archived"
    );
}

// ── 5. terminate_stale_child: active parent → kill + notification ────────

/// When a stale child is terminated and the parent is active, the child
/// is killed and a `Terminated` announce event is pushed to the parent.
#[tokio::test]
#[serial]
async fn test_terminate_stale_child_active_parent_notifies() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-kill1").await;

    let child_id = mgr
        .create_child_session(
            &super::test_helpers::test_resolved_config("worker-kill1", None),
            &parent_id,
            1,
            "stale work",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .expect("create_child_session");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text("partial".into())],
    )
    .await;

    mgr.terminate_stale_child(&parent_id, &child_id).await;

    // Child should be gone from children table.
    let children = mgr.children.read().await;
    assert!(
        children.find_child(&child_id).is_none(),
        "stale child should be removed from children table"
    );
    drop(children);

    // Parent should have a Terminated announce in its queue.
    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 Terminated announce");
    assert_eq!(drained[0].child_session_id, child_id);
    assert_eq!(
        drained[0].status,
        closeclaw_common::ChildCompletionStatus::Terminated
    );
    assert!(
        drained[0].result_text.contains("僵死"),
        "result text should mention stale, got: {}",
        drained[0].result_text
    );
    assert!(
        drained[0].result_text.contains("300"),
        "result text should mention threshold (300s), got: {}",
        drained[0].result_text
    );
    assert_eq!(drained[0].priority, NotificationPriority::Next);
}

// ── 6. terminate_stale_child: archived parent → kill + no notification ──

/// When the parent session is archived (not in sessions map), the child
/// is still killed but no announce is pushed.
#[tokio::test]
#[serial]
async fn test_terminate_stale_child_archived_parent_no_notification() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-kill2").await;

    let child_id = mgr
        .create_child_session(
            &super::test_helpers::test_resolved_config("worker-kill2", None),
            &parent_id,
            1,
            "stale work 2",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .expect("create_child_session");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text("partial2".into())],
    )
    .await;

    // Remove the parent from sessions to simulate archived state.
    mgr.sessions.write().await.remove(&parent_id);

    mgr.terminate_stale_child(&parent_id, &child_id).await;

    // Child should be gone from children table.
    let children = mgr.children.read().await;
    assert!(
        children.find_child(&child_id).is_none(),
        "stale child should still be removed when parent archived"
    );
    drop(children);

    // No conversation session for parent → push_announce would fail,
    // but the impl checks is_parent_archived first and skips.
    let cs = mgr.get_conversation_session(&parent_id).await;
    // Parent may still have a conversation session; drain to verify.
    if let Some(cs) = cs {
        let mut cs = cs.write().await;
        // The announce queue should NOT have a Terminated event.
        // drain_all_entries returns all queued items.
        let entries = cs.drain_all_entries();
        let has_terminated = entries.iter().any(|e| match e {
            closeclaw_session::llm_session::QueueEntry::Announce(a) => {
                a.status == closeclaw_common::ChildCompletionStatus::Terminated
            }
            _ => false,
        });
        assert!(
            !has_terminated,
            "no Terminated announce should be pushed when parent archived"
        );
    }
}

// ── 7. terminate_stale_child: non-existent child → no panic ─────────────

/// Terminating a child id that doesn't exist in the children table
/// should not panic. `kill_child` returns `Err` which is handled
/// gracefully.
#[tokio::test]
#[serial]
async fn test_terminate_stale_child_nonexistent_no_panic() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let _parent_id = setup_parent_with_conv(&mgr, "parent-kill3").await;

    // Should not panic even with a non-existent child.
    mgr.terminate_stale_child("parent-kill3", "nonexistent-child")
        .await;
}

// ── 8. terminate_stale_child: cascade kills nested descendants ──────────

/// A stale child that has nested descendants (grandchild, etc.) should
/// have all descendants cascade-killed when terminated.
#[tokio::test]
#[serial]
async fn test_terminate_stale_child_cascade_descendants() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-casc").await;

    // Create child and grandchild.
    let child_id = mgr
        .create_child_session(
            &super::test_helpers::test_resolved_config("worker-casc", None),
            &parent_id,
            1,
            "stale parent",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .expect("create_child_session");

    let grandchild_id = mgr
        .create_child_session(
            &super::test_helpers::test_resolved_config("worker-casc-gc", None),
            &child_id,
            2,
            "nested work",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .expect("create grandchild");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text(
            "child done".into(),
        )],
    )
    .await;
    append_assistant_to_child(
        &mgr,
        &grandchild_id,
        vec![closeclaw_llm::types::ContentBlock::Text("gc done".into())],
    )
    .await;

    // Verify both exist before termination.
    assert!(mgr.children.read().await.find_child(&child_id).is_some());
    assert!(mgr
        .children
        .read()
        .await
        .find_child(&grandchild_id)
        .is_some());

    mgr.terminate_stale_child(&parent_id, &child_id).await;

    // Both child and grandchild should be gone.
    let children = mgr.children.read().await;
    assert!(
        children.find_child(&child_id).is_none(),
        "stale child should be removed"
    );
    assert!(
        children.find_child(&grandchild_id).is_none(),
        "grandchild should be cascade-removed"
    );
}

// ── 9. terminate_stale_child: announcement text format ───────────────────

/// The Terminated announce text must contain the stale duration (300s)
/// and indicate the child was auto-terminated.
#[tokio::test]
#[serial]
async fn test_terminate_stale_child_notification_text_format() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-fmt").await;

    let child_id = mgr
        .create_child_session(
            &super::test_helpers::test_resolved_config("worker-fmt", None),
            &parent_id,
            1,
            "format test",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .expect("create_child_session");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text("test".into())],
    )
    .await;

    mgr.terminate_stale_child(&parent_id, &child_id).await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1);

    let text = &drained[0].result_text;
    assert!(
        text.contains("僵死"),
        "should mention '僵死' in result text: {}",
        text
    );
    assert!(
        text.contains("300"),
        "should mention threshold 300s: {}",
        text
    );
    assert!(
        text.contains("自动终止"),
        "should mention auto-terminated: {}",
        text
    );
}

// ── 10. is_session_idle: background tool running → true ────────────────

/// `is_session_idle` returns `true` when a background tool is running.
/// This verifies the core fix from Step 1.1: background_tool_active
/// no longer blocks idle determination (design doc state table row 2).
#[tokio::test]
#[serial]
async fn test_is_session_idle_with_background_tool() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let sid = mgr.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Register a background tool via the ToolSession trait.
    let cs = mgr
        .get_conversation_session(&sid)
        .await
        .expect("session exists");
    {
        let guard = cs.write().await;
        <closeclaw_session::llm_session::ConversationSession as ToolSession>::register_tool_call(
            &*guard,
            "bg-sweep-1".into(),
            "bash".into(),
            "ls".into(),
        )
        .await;
        <closeclaw_session::llm_session::ConversationSession as ToolSession>::update_tool_state(
            &*guard,
            "bg-sweep-1",
            ToolExecState::RunningBackground,
        )
        .await;
    }

    // Background tool running → session IS idle (per design doc).
    assert!(
        mgr.is_session_idle(&sid).await,
        "is_session_idle must return true when only background tool is active"
    );
}
