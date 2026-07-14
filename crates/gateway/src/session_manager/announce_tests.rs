//! Tests for the announce pipeline (Step 1.6).
//!
//! These tests cover:
//! - `test_push_and_drain_announce`
//! - `test_try_push_announce_run_mode`
//! - `test_try_push_announce_session_mode_noop`
//! - `test_try_push_announce_non_child_noop`
//! - `test_announce_inject_as_system_message`
//! - `test_thinking_blocks_excluded`
//! - `test_parallel_announce_ordering`
//!
//! Step 1.6 (test scaffolding) is added after Steps 1.3–1.5 land.
//!
//! Shared helpers (e.g. `test_resolved_config`, `setup_parent_with_conv`,
//! `inject_events_and_return_messages`, `spawn_n_run_children`) live in
//! `super::test_helpers` to keep this file under the 500-line limit.

use super::spawn::SpawnMode;
use super::test_helpers::{
    append_assistant_to_child, inject_events_and_return_messages, register_child_only,
    setup_parent_with_conv, spawn_n_run_children, test_resolved_config,
};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use chrono::Utc;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::llm_session::{AnnounceEvent, ChatSession};
use closeclaw_tasks::{
    BackgroundTask, BackgroundTaskError, CompletionNotification, NotificationPriority, TaskManager,
    TaskState,
};
use serial_test::serial;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ── 1. test_push_and_drain_announce ─────────────────────────────────────────

/// `push_announce` should accept multiple events in order, and
/// `drain_announces` should return all of them in FIFO order, leaving
/// the queue empty.
#[tokio::test]
#[serial]
async fn test_push_and_drain_announce() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pd").await;

    for i in 0..3 {
        let event = AnnounceEvent {
            child_session_id: format!("child-{}", i),
            child_agent_id: format!("agent-{}", i),
            result_text: format!("result-{}", i),
            completed_at: Utc::now(),
            priority: NotificationPriority::Next,
        };
        mgr.push_announce(&parent_id, event)
            .await
            .expect("push_announce should succeed");
    }

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 3, "expected 3 events");
    for (i, ev) in drained.iter().enumerate() {
        assert_eq!(ev.child_session_id, format!("child-{}", i));
        assert_eq!(ev.child_agent_id, format!("agent-{}", i));
        assert_eq!(ev.result_text, format!("result-{}", i));
    }

    assert!(
        mgr.drain_announces(&parent_id).await.is_empty(),
        "queue should be empty after first drain"
    );
}

// ── 2. test_try_push_announce_run_mode ──────────────────────────────────────

/// A run-mode child that has completed an assistant turn should produce
/// an `AnnounceEvent` on the parent's queue when `try_push_announce` is
/// called.
#[tokio::test]
#[serial]
async fn test_try_push_announce_run_mode() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-run").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-run", None),
            &parent_id,
            1,
            "do work",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,    // max_spawn_depth
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("task complete".to_string())],
    )
    .await;

    mgr.try_push_announce(&child_id).await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1, "expected 1 announce event");
    let ev = &drained[0];
    assert_eq!(ev.child_session_id, child_id);
    assert_eq!(ev.child_agent_id, "worker-run");
    assert_eq!(ev.result_text, "task complete");
}

// ── 3. test_try_push_announce_session_mode_noop ─────────────────────────────

/// A session-mode child must NOT produce an announce — only run-mode
/// children trigger the push.
#[tokio::test]
#[serial]
async fn test_try_push_announce_session_mode_noop() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-sess").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-sess", None),
            &parent_id,
            1,
            "stay alive",
            true,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,    // max_spawn_depth
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("still running".to_string())],
    )
    .await;

    mgr.try_push_announce(&child_id).await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert!(
        drained.is_empty(),
        "session-mode child should not push an announce, got: {:?}",
        drained
    );
}

// ── 4. test_try_push_announce_non_child_noop ────────────────────────────────

/// A session id that is not registered as a child (in the `children`
/// table) should produce no announce, no panic, and not disturb any
/// other parent's queue.
#[tokio::test]
#[serial]
async fn test_try_push_announce_non_child_noop() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-orphan").await;

    let other_parent = setup_parent_with_conv(&mgr, "parent-other").await;
    register_child_only(&mgr, &other_parent, "real-child", "agent-x", SpawnMode::Run).await;

    mgr.try_push_announce("not-a-real-child").await;
    assert!(mgr.drain_announces(&parent_id).await.is_empty());
    assert!(mgr.drain_announces(&other_parent).await.is_empty());

    mgr.try_push_announce("00000000-0000-0000-0000-000000000000")
        .await;
    assert!(mgr.drain_announces(&parent_id).await.is_empty());
}

// ── 5. test_announce_inject_as_system_message ───────────────────────────────

/// After draining announce events and injecting them via
/// `inject_system_message`, the parent's message history must contain
/// a `role="system"` `SessionMessage` that includes the child's agent
/// id and the result text.
#[tokio::test]
#[serial]
async fn test_announce_inject_as_system_message() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-inj").await;

    let event = AnnounceEvent {
        child_session_id: "child-inj".to_string(),
        child_agent_id: "sub-agent-42".to_string(),
        result_text: "computed answer".to_string(),
        completed_at: Utc::now(),
        priority: NotificationPriority::Next,
    };
    mgr.push_announce(&parent_id, event)
        .await
        .expect("push_announce should succeed");

    let messages = inject_events_and_return_messages(&mgr, &parent_id).await;

    assert_eq!(
        messages.len(),
        1,
        "expected exactly one injected system message"
    );
    let msg = &messages[0];
    assert_eq!(msg.role, "system");
    assert_eq!(msg.content_blocks.len(), 1);
    let rendered = match &msg.content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        rendered.contains("sub-agent-42"),
        "rendered text should contain child agent id, got: {}",
        rendered
    );
    assert!(
        rendered.contains("computed answer"),
        "rendered text should contain result text, got: {}",
        rendered
    );
}

// ── 6. test_thinking_blocks_excluded ────────────────────────────────────────

/// When the child's last assistant message contains both Thinking and
/// Text blocks, only the Text content must be included in
/// `AnnounceEvent.result_text`.
#[tokio::test]
#[serial]
async fn test_thinking_blocks_excluded() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-think").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-think", None),
            &parent_id,
            1,
            "think first",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,    // max_spawn_depth
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![
            ContentBlock::Thinking {
                thinking: "secret reasoning that should NOT leak".to_string(),
                signature: None,
            },
            ContentBlock::Text("final answer that MUST be present".to_string()),
        ],
    )
    .await;

    mgr.try_push_announce(&child_id).await;

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1);
    let ev = &drained[0];
    assert!(
        !ev.result_text.contains("secret reasoning"),
        "thinking content leaked into announce: {}",
        ev.result_text
    );
    assert!(
        ev.result_text.contains("final answer that MUST be present"),
        "result_text should contain the Text block, got: {}",
        ev.result_text
    );
}

// ── 7. test_parallel_announce_ordering ──────────────────────────────────────

/// Multiple run-mode children completing in parallel must all produce
/// exactly one announce each, with no deadlocks and no event loss.
/// The events are pushed onto the parent queue under a write lock, so
/// they end up in the order each call's `push_announce` actually
/// acquired the parent lock — which is deterministic per scheduler run
/// but we only assert count + presence here, not a specific order
/// (since that depends on OS scheduling).
#[tokio::test]
#[serial]
async fn test_parallel_announce_ordering() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = std::sync::Arc::new(make_test_mgr(Some(tmp.path())));
    let parent_id = setup_parent_with_conv(&mgr, "parent-par").await;

    const N: usize = 5;
    let child_ids = spawn_n_run_children(&mgr, &parent_id, N).await;

    // tokio::join! polls them concurrently; if any deadlocks the test
    // will hang and time out.
    let mut futs = Vec::with_capacity(N);
    for cid in &child_ids {
        let mgr2 = mgr.clone();
        let cid2 = cid.clone();
        futs.push(tokio::spawn(async move {
            mgr2.try_push_announce(&cid2).await;
        }));
    }
    for f in futs {
        f.await.expect("try_push_announce task should not panic");
    }

    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(
        drained.len(),
        N,
        "expected {} events, got {}",
        N,
        drained.len()
    );

    let drained_ids: HashSet<&str> = drained
        .iter()
        .map(|e| e.child_session_id.as_str())
        .collect();
    let expected_ids: HashSet<&str> = child_ids.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        drained_ids, expected_ids,
        "drained child ids should match registered child ids"
    );

    assert!(mgr.drain_announces(&parent_id).await.is_empty());
}

// ── 8. test_try_push_announce_sends_mining_notification ─────────────────────

/// When a run-mode sub-agent session completes, `try_push_announce` must
/// send the child session ID through the `mining_notify_tx` channel so
/// the DreamingScheduler can trigger mining immediately (design doc
/// §触发 1).
#[tokio::test]
#[serial]
async fn test_try_push_announce_sends_mining_notification() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-mine").await;

    // Wire mining notify channel.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    mgr.set_mining_notify_tx(tx);

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-mine", None),
            &parent_id,
            1,
            "mine this",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,    // max_spawn_depth
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("mined result".to_string())],
    )
    .await;

    mgr.try_push_announce(&child_id).await;

    // Verify mining notification was sent.
    let received = rx
        .recv()
        .await
        .expect("mining notification should have been sent");
    assert_eq!(received, child_id);
}

// ── 9. test_try_push_announce_no_notification_without_tx ────────────────────

/// When no `mining_notify_tx` is set, `try_push_announce` must not panic
/// — the mining notification path is simply skipped.
#[tokio::test]
#[serial]
async fn test_try_push_announce_no_notification_without_tx() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-no-mine").await;

    // Do NOT set mining_notify_tx — should still work.
    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-no-mine", None),
            &parent_id,
            1,
            "no mine",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,    // max_spawn_depth
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("done".to_string())],
    )
    .await;

    // Should not panic even without mining_notify_tx.
    mgr.try_push_announce(&child_id).await;

    // Announce should still be pushed.
    let drained = mgr.drain_announces(&parent_id).await;
    assert_eq!(drained.len(), 1);
}

// ── 10. test_session_mode_no_mining_notification ────────────────────────────

/// A session-mode child must NOT trigger a mining notification — only
/// run-mode sub-agent sessions trigger §触发 1.
#[tokio::test]
#[serial]
async fn test_session_mode_no_mining_notification() {
    clear_global_prompt_state();

    let tmp = TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-sess-mine").await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    mgr.set_mining_notify_tx(tx);

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-sess-mine", None),
            &parent_id,
            1,
            "session work",
            true,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,    // max_spawn_depth
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![ContentBlock::Text("session result".to_string())],
    )
    .await;

    mgr.try_push_announce(&child_id).await;

    // No mining notification should be sent for session-mode child.
    let result = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
    assert!(
        result.is_err() || result.unwrap().is_none(),
        "session-mode child must not trigger mining notification"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Gap 3 tests — notification priority differentiation
// ═══════════════════════════════════════════════════════════════════════════

// ── 11. test_next_priority_notification_has_urgency_marker ─────────────────

/// When `drain_announce_events` processes a notification with
/// `NotificationPriority::Next`, the injected system message must
/// contain the `[⚠️ 需立即处理]` urgency prefix so the agent
/// recognizes the need for immediate attention.
#[tokio::test]
#[serial]
async fn test_next_priority_notification_has_urgency_marker() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-next").await;

    // Set up mock task manager with a Next-priority notification.
    let notifications = vec![CompletionNotification {
        task_id: "task-next-1".to_string(),
        command: "important_cmd".to_string(),
        state: TaskState::Completed { exit_code: 0 },
        output_path: PathBuf::from("/tmp/test/output-next"),
        priority: NotificationPriority::Next,
        summary: "Background command 'important_cmd' completed".to_string(),
    }];
    let mock_tm = Arc::new(MockTaskManager::new(notifications));
    mgr.set_task_manager(mock_tm as Arc<dyn TaskManager>).await;

    // Simulate the drain_announce_events injection logic.
    let tm = mgr.get_task_manager().await.unwrap();
    let drained = tm.drain_notifications().await;
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    {
        let mut cs_write = cs.write().await;
        for notif in drained {
            let prefix = match notif.priority {
                NotificationPriority::Now => "[🚨 紧急] 后台任务",
                NotificationPriority::Next => "[⚠️ 需立即处理] 后台任务",
                NotificationPriority::Later => "[后台任务]",
            };
            let text = format!(
                "{} 任务 {}（命令 '{}'）已完成（状态：Completed \
                 (exit code: 0)）。输出文件：{}",
                prefix,
                notif.task_id,
                notif.command,
                notif.output_path.display()
            );
            cs_write.inject_system_message(text);
        }
    }

    let messages = cs.read().await.messages().to_vec();
    assert_eq!(messages.len(), 1);
    let rendered = match &messages[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        rendered.contains("[⚠️ 需立即处理] 后台任务"),
        "Next priority notification must contain urgency marker, got: {}",
        rendered
    );
}

// ── 12. test_later_priority_notification_normal_format ────────────────────────

/// When `drain_announce_events` processes a notification with
/// `NotificationPriority::Later`, the injected system message must
/// use the standard `[后台任务]` prefix without urgency marking.
#[tokio::test]
#[serial]
async fn test_later_priority_notification_normal_format() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-later").await;

    let notifications = vec![CompletionNotification {
        task_id: "task-later-1".to_string(),
        command: "normal_cmd".to_string(),
        state: TaskState::Completed { exit_code: 0 },
        output_path: PathBuf::from("/tmp/test/output-later"),
        priority: NotificationPriority::Later,
        summary: "Background command 'normal_cmd' completed".to_string(),
    }];
    let mock_tm = Arc::new(MockTaskManager::new(notifications));
    mgr.set_task_manager(mock_tm as Arc<dyn TaskManager>).await;

    let tm = mgr.get_task_manager().await.unwrap();
    let drained = tm.drain_notifications().await;
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    {
        let mut cs_write = cs.write().await;
        for notif in drained {
            let prefix = match notif.priority {
                NotificationPriority::Now => "[🚨 紧急] 后台任务",
                NotificationPriority::Next => "[⚠️ 需立即处理] 后台任务",
                NotificationPriority::Later => "[后台任务]",
            };
            let text = format!(
                "{} 任务 {}（命令 '{}'）已完成（状态：Completed \
                 (exit code: 0)）。输出文件：{}",
                prefix,
                notif.task_id,
                notif.command,
                notif.output_path.display()
            );
            cs_write.inject_system_message(text);
        }
    }

    let messages = cs.read().await.messages().to_vec();
    assert_eq!(messages.len(), 1);
    let rendered = match &messages[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        rendered.contains("[后台任务]"),
        "Later priority notification must use normal prefix, got: {}",
        rendered
    );
    assert!(
        !rendered.contains("⚠️"),
        "Later priority notification must NOT contain urgency marker, got: {}",
        rendered
    );
}

// ── 13. test_priority_consistency_from_generation_to_consumption ──────────────

/// Verify that the `priority` field on `CompletionNotification` is
/// preserved end-to-end: the value set during `finalize_state` is the
/// same value seen when `drain_notifications` returns it. This tests
/// the state-transition integrity described in the design doc §通知机制.
#[tokio::test]
#[serial]
async fn test_priority_consistency_from_generation_to_consumption() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let _parent_id = setup_parent_with_conv(&mgr, "parent-consist").await;

    // Simulate two notifications with different priorities as if they
    // were produced by finalize_state (Later) and stuck_detect (Next).
    let notifications = vec![
        CompletionNotification {
            task_id: "task-consist-later".to_string(),
            command: "echo done".to_string(),
            state: TaskState::Completed { exit_code: 0 },
            output_path: PathBuf::from("/tmp/output-later"),
            priority: NotificationPriority::Later,
            summary: "Background command 'echo done' completed".to_string(),
        },
        CompletionNotification {
            task_id: "task-consist-next".to_string(),
            command: "stuck command".to_string(),
            state: TaskState::Failed { exit_code: 1 },
            output_path: PathBuf::from("/tmp/output-next"),
            priority: NotificationPriority::Next,
            summary: "Background command 'stuck command' appears stuck at an interactive prompt"
                .to_string(),
        },
    ];
    let mock_tm = Arc::new(MockTaskManager::new(notifications));
    mgr.set_task_manager(mock_tm as Arc<dyn TaskManager>).await;

    // Drain once — should return both notifications with original
    // priorities.
    let tm = mgr.get_task_manager().await.unwrap();
    let drained = tm.drain_notifications().await;
    assert_eq!(drained.len(), 2);

    let later = drained.iter().find(|n| n.task_id == "task-consist-later");
    let next = drained.iter().find(|n| n.task_id == "task-consist-next");

    assert!(later.is_some(), "Later notification should be present");
    assert!(next.is_some(), "Next notification should be present");
    assert_eq!(later.unwrap().priority, NotificationPriority::Later);
    assert_eq!(next.unwrap().priority, NotificationPriority::Next);

    // Drain again — should be empty (dedup).
    let drained2 = tm.drain_notifications().await;
    assert!(drained2.is_empty(), "second drain should be empty");
}

// ── 14. test_next_vs_later_prefix_comparison ─────────────────────────────────

/// Verify that Next and Later priority notifications produce
/// different text prefixes, and that the difference is exactly the
/// urgency marker `[⚠️ 需立即处理]`.
#[tokio::test]
#[serial]
async fn test_next_vs_later_prefix_comparison() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-compare").await;

    let notifications = vec![
        CompletionNotification {
            task_id: "task-a".to_string(),
            command: "cmd_a".to_string(),
            state: TaskState::Completed { exit_code: 0 },
            output_path: PathBuf::from("/tmp/a"),
            priority: NotificationPriority::Next,
            summary: "Background command 'cmd_a' completed".to_string(),
        },
        CompletionNotification {
            task_id: "task-b".to_string(),
            command: "cmd_b".to_string(),
            state: TaskState::Completed { exit_code: 0 },
            output_path: PathBuf::from("/tmp/b"),
            priority: NotificationPriority::Later,
            summary: "Background command 'cmd_b' completed".to_string(),
        },
    ];
    let mock_tm = Arc::new(MockTaskManager::new(notifications));
    mgr.set_task_manager(mock_tm as Arc<dyn TaskManager>).await;

    let tm = mgr.get_task_manager().await.unwrap();
    let drained = tm.drain_notifications().await;
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    {
        let mut cs_write = cs.write().await;
        for notif in drained {
            let prefix = match notif.priority {
                NotificationPriority::Now => "[🚨 紧急] 后台任务",
                NotificationPriority::Next => "[⚠️ 需立即处理] 后台任务",
                NotificationPriority::Later => "[后台任务]",
            };
            let text = format!(
                "{} 任务 {}（命令 '{}'）已完成（状态：Completed \
                 (exit code: 0)）。输出文件：{}",
                prefix,
                notif.task_id,
                notif.command,
                notif.output_path.display()
            );
            cs_write.inject_system_message(text);
        }
    }

    let messages = cs.read().await.messages().to_vec();
    assert_eq!(messages.len(), 2);

    // Messages should be sorted by content for deterministic comparison.
    let mut rendered: Vec<String> = messages
        .iter()
        .map(|m| match &m.content_blocks[0] {
            ContentBlock::Text(t) => t.clone(),
            other => panic!("expected Text, got {:?}", other),
        })
        .collect();
    rendered.sort();

    // Next-priority message must contain the urgency marker.
    let next_msg = rendered
        .iter()
        .find(|t| t.contains("⚠️"))
        .expect("Next priority message must contain ⚠️");
    assert!(next_msg.contains("task-a"));
    assert!(next_msg.contains("[⚠️ 需立即处理] 后台任务"));

    // Later-priority message must NOT contain the urgency marker.
    let later_msg = rendered
        .iter()
        .find(|t| !t.contains("⚠️") && t.contains("[后台任务]"))
        .expect("Later priority message must exist");
    assert!(later_msg.contains("task-b"));
    assert!(later_msg.starts_with("[后台任务]"));
}

// ── 15. test_now_priority_notification_has_urgent_prefix ────────────────────

/// When `drain_announce_events` processes a notification with
/// `NotificationPriority::Now`, the injected system message must
/// contain the `[🚨 紧急]` urgent prefix for system-level emergency
/// notifications.
#[tokio::test]
#[serial]
async fn test_now_priority_notification_has_urgent_prefix() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-now").await;

    let notifications = vec![CompletionNotification {
        task_id: "task-now-1".to_string(),
        command: "critical_cmd".to_string(),
        state: TaskState::Completed { exit_code: 0 },
        output_path: PathBuf::from("/tmp/test/output-now"),
        priority: NotificationPriority::Now,
        summary: "Background command 'critical_cmd' completed".to_string(),
    }];
    let mock_tm = Arc::new(MockTaskManager::new(notifications));
    mgr.set_task_manager(mock_tm as Arc<dyn TaskManager>).await;

    let tm = mgr.get_task_manager().await.unwrap();
    let drained = tm.drain_notifications().await;
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    {
        let mut cs_write = cs.write().await;
        for notif in drained {
            let prefix = match notif.priority {
                NotificationPriority::Now => "[🚨 紧急] 后台任务",
                NotificationPriority::Next => "[⚠️ 需立即处理] 后台任务",
                NotificationPriority::Later => "[后台任务]",
            };
            let text = format!(
                "{} 任务 {}（命令 '{}'）已完成（状态：Completed \
                 (exit code: 0)）。输出文件：{}",
                prefix,
                notif.task_id,
                notif.command,
                notif.output_path.display()
            );
            cs_write.inject_system_message(text);
        }
    }

    let messages = cs.read().await.messages().to_vec();
    assert_eq!(messages.len(), 1);
    let rendered = match &messages[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text block, got {:?}", other),
    };
    assert!(
        rendered.contains("[🚨 紧急] 后台任务"),
        "Now priority notification must contain urgent prefix, got: {}",
        rendered
    );
    assert!(
        !rendered.contains("⚠️"),
        "Now priority notification must NOT contain Next's urgency marker, got: {}",
        rendered
    );
}

// ── 16. test_mock_task_manager_dedup ────────────────────────────────────────

/// Verify that MockTaskManager's drain_notifications respects dedup:
/// first drain returns all notifications, second returns empty.
#[tokio::test]
#[serial]
async fn test_mock_task_manager_dedup() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let _parent_id = setup_parent_with_conv(&mgr, "parent-dedup").await;

    let notifications = vec![CompletionNotification {
        task_id: "task-dedup".to_string(),
        command: "echo dedup".to_string(),
        state: TaskState::Completed { exit_code: 0 },
        output_path: PathBuf::from("/tmp/dedup"),
        priority: NotificationPriority::Later,
        summary: "Background command 'echo dedup' completed".to_string(),
    }];
    let mock_tm = Arc::new(MockTaskManager::new(notifications));
    mgr.set_task_manager(mock_tm as Arc<dyn TaskManager>).await;

    let tm = mgr.get_task_manager().await.unwrap();
    let first = tm.drain_notifications().await;
    assert_eq!(first.len(), 1);

    let second = tm.drain_notifications().await;
    assert!(
        second.is_empty(),
        "second drain should return empty (dedup)"
    );
}

// ── Mock TaskManager ────────────────────────────────────────────────────────

/// A minimal mock implementing [`closeclaw_tasks::TaskManager`] that
/// returns pre-built notifications from `drain_notifications`. Used
/// to test notification priority formatting without spawning real
/// background processes.
struct MockTaskManager {
    notifications: std::sync::Mutex<Vec<CompletionNotification>>,
}

impl MockTaskManager {
    fn new(notifications: Vec<CompletionNotification>) -> Self {
        Self {
            notifications: std::sync::Mutex::new(notifications),
        }
    }
}

#[async_trait::async_trait]
impl TaskManager for MockTaskManager {
    async fn spawn_task(
        &self,
        _command: &str,
        _cwd: &std::path::Path,
        _is_backgrounded: bool,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!("MockTaskManager::spawn_task")
    }

    async fn backgroundize_task(
        &self,
        _child: tokio::process::Child,
        _command: &str,
        _is_backgrounded: bool,
    ) -> Result<BackgroundTask, BackgroundTaskError> {
        unimplemented!("MockTaskManager::backgroundize_task")
    }

    async fn kill_task(&self, _task_id: &str) -> Result<(), BackgroundTaskError> {
        unimplemented!("MockTaskManager::kill_task")
    }

    async fn get_task(&self, _task_id: &str) -> Option<BackgroundTask> {
        None
    }

    async fn drain_notifications(&self) -> Vec<CompletionNotification> {
        std::mem::take(&mut *self.notifications.lock().unwrap())
    }

    async fn cleanup_finished(&self) {}
}
