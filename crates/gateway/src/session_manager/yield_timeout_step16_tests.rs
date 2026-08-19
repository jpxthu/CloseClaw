//! Additional yield timeout tests for Step 1.6.
//!
//! Covers structured notification content, no-force-terminate behavior,
//! per-child spawn timeout independence, ChildSessionInfo field validation,
//! and created_at timestamp tracking.

use super::spawn::SpawnMode;
use super::test_helpers::{setup_parent_with_conv, test_resolved_config};
use super::tests::clear_global_prompt_state;
use closeclaw_session::llm_session::ChatSession;
use serial_test::serial;
use std::sync::Arc;

/// Shorthand for creating a SessionManager in tests.
fn mgr() -> Arc<super::SessionManager> {
    Arc::new(super::tests::make_test_mgr(None))
}

// ── 16. Structured notification lists child status and elapsed time ──────

/// Verify that the timeout notification lists each child's session ID,
/// status (completed/running), and elapsed execution time.
#[tokio::test]
#[serial]
async fn test_yield_timeout_structured_notification_content() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-sn").await;

    // Spawn two children: one will be marked Completed, one remains Active.
    let child1_id = m
        .create_child_session(
            &test_resolved_config("worker-sn1", None),
            &parent_id,
            1,
            "task 1",
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
        .unwrap();

    let child2_id = m
        .create_child_session(
            &test_resolved_config("worker-sn2", None),
            &parent_id,
            1,
            "task 2",
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
        .unwrap();

    // Mark child1 as Completed in the parent's child_states.
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        let guard = cs.read().await;
        guard.update_child_state(&child1_id, closeclaw_common::ChildSessionState::Completed);
    }

    // Enter Waiting and start a 1-second timeout.
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;

    // Wait for timeout to fire.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Verify the structured notification content.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let timeout_msg = messages.iter().find(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(
                    b,
                    closeclaw_llm::types::ContentBlock::Text(t) if t.contains("等待上限")
                )
            })
    });
    assert!(
        timeout_msg.is_some(),
        "timeout notification should be present in transcript"
    );

    let msg_text = timeout_msg
        .unwrap()
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            closeclaw_llm::types::ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    // Should list child IDs.
    assert!(
        msg_text.contains(&child1_id),
        "notification should mention child1 ID"
    );
    assert!(
        msg_text.contains(&child2_id),
        "notification should mention child2 ID"
    );

    // Should show completed status for child1.
    assert!(
        msg_text.contains("已完成"),
        "notification should show completed status"
    );

    // Should show running status for child2.
    assert!(
        msg_text.contains("运行中"),
        "notification should show running status"
    );

    // Should show elapsed seconds for at least one child.
    assert!(
        msg_text.contains("已运行"),
        "notification should mention elapsed time"
    );
}

// ── 17. No force-terminate after overall timeout ──────────────────────────

/// After the overall yield timeout fires, child sessions should still
/// be Active (not terminated). Per-child spawn timeouts handle
/// individual termination.
#[tokio::test]
#[serial]
async fn test_yield_timeout_no_force_terminate_children() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-nft").await;

    // Spawn two children (with long timeouts so they don't hit per-child timeout).
    let _child1 = m
        .create_child_session(
            &test_resolved_config("worker-nft1", None),
            &parent_id,
            1,
            "task 1",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            Some(600), // long per-child timeout
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    let _child2 = m
        .create_child_session(
            &test_resolved_config("worker-nft2", None),
            &parent_id,
            1,
            "task 2",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            Some(600), // long per-child timeout
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting and start a 1-second overall timeout.
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;

    // Wait for overall timeout to fire.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(
        !m.is_session_yielding(&parent_id).await,
        "session should exit Waiting after overall timeout"
    );

    // Children should still be Active (not terminated by yield timeout).
    let children = m.children.read().await;
    let child_list = children.list_children(&parent_id);
    assert_eq!(child_list.len(), 2, "both children should still exist");
    for info in &child_list {
        assert_eq!(
            info.status,
            super::spawn::ChildSessionStatus::Active,
            "child {} should still be Active after yield timeout",
            info.session_id
        );
    }
}

// ── 18. Per-child spawn timeout is independent ───────────────────────────

/// Per-child spawn timeout still force-stops the individual child
/// independently of the yield timeout.
#[tokio::test]
#[serial]
async fn test_yield_per_child_spawn_timeout_independent() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-pct").await;

    // Spawn a child with a very short per-child timeout (1 second).
    let _child_id = m
        .create_child_session(
            &test_resolved_config("worker-pct", None),
            &parent_id,
            1,
            "quick task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            Some(1), // 1-second per-child timeout
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Enter Waiting and start a long yield timeout (30 seconds).
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 30, None, None)
        .await;

    // Wait for per-child timeout to fire (1s + buffer).
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // The per-child timeout should have injected an announce event.
    let drained = m.drain_announces(&parent_id).await;
    let has_child_timeout = drained
        .iter()
        .any(|ev| ev.result_text.contains("spawn timeout") && ev.result_text.contains("exceeded"));
    assert!(
        has_child_timeout,
        "per-child spawn timeout should have injected announce event"
    );

    // Cleanup: cancel yield timeout.
    m.cancel_yield_timeout(&parent_id).await;
}

// ── 19. ChildSessionInfo.timeout_secs correctly passed ────────────────────

/// Verify that `timeout_secs` in `ChildSessionInfo` is correctly set
/// from the `spawn_timeout` parameter.
#[tokio::test]
#[serial]
async fn test_child_session_info_timeout_secs_passed() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-tsi").await;

    // Spawn with explicit timeout.
    let child_id = m
        .create_child_session(
            &test_resolved_config("worker-tsi", None),
            &parent_id,
            1,
            "task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            Some(120), // explicit timeout
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    // Verify timeout_secs is set.
    let children = m.children.read().await;
    let child_info = children
        .list_children(&parent_id)
        .into_iter()
        .find(|c| c.session_id == child_id)
        .expect("child should be registered");
    assert_eq!(
        child_info.timeout_secs,
        Some(120),
        "ChildSessionInfo.timeout_secs should match spawn_timeout"
    );
}

/// Verify that `timeout_secs` is None when no spawn_timeout is provided.
#[tokio::test]
#[serial]
async fn test_child_session_info_timeout_secs_none() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-ts2").await;

    // Spawn without timeout.
    let child_id = m
        .create_child_session(
            &test_resolved_config("worker-ts2", None),
            &parent_id,
            1,
            "task",
            true,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None, // no timeout
            None,
            None,
            None, // timeout_warning_secs
            None, // timeout_notify_interval_ratio
        )
        .await
        .unwrap();

    let children = m.children.read().await;
    let child_info = children
        .list_children(&parent_id)
        .into_iter()
        .find(|c| c.session_id == child_id)
        .expect("child should be registered");
    assert_eq!(
        child_info.timeout_secs, None,
        "ChildSessionInfo.timeout_secs should be None when not specified"
    );
}

// ── 20. Created_at records child creation time ───────────────────────────

/// Verify that `ChildSessionInfo.created_at` records the child's creation
/// time and is used by the structured notification to compute elapsed time.
#[tokio::test]
#[serial]
async fn test_child_session_info_created_at() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-ca").await;

    let before = std::time::Instant::now();
    let child_id = m
        .create_child_session(
            &test_resolved_config("worker-ca", None),
            &parent_id,
            1,
            "task",
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
        .unwrap();
    let after = std::time::Instant::now();

    let children = m.children.read().await;
    let child_info = children
        .list_children(&parent_id)
        .into_iter()
        .find(|c| c.session_id == child_id)
        .expect("child should be registered");

    // created_at should be between before and after.
    assert!(
        child_info.created_at >= before,
        "created_at should be >= creation start"
    );
    assert!(
        child_info.created_at <= after,
        "created_at should be <= creation end"
    );
}

// ── 21. Yield timeout with multiple children ─────────────────────────────

/// Verify that `start_yield_timeout` works correctly with multiple
/// children registered under the parent.
#[tokio::test]
#[serial]
async fn test_yield_timeout_with_multiple_children() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-mc").await;

    // Spawn two children with different timeouts.
    m.create_child_session(
        &test_resolved_config("worker-mc1", None),
        &parent_id,
        1,
        "task 1",
        true,
        None,
        SpawnMode::Run,
        false,
        None,
        None,
        None,
        3,
        Some(100),
        None,
        None,
        None, // timeout_warning_secs
        None, // timeout_notify_interval_ratio
    )
    .await
    .unwrap();

    m.create_child_session(
        &test_resolved_config("worker-mc2", None),
        &parent_id,
        1,
        "task 2",
        true,
        None,
        SpawnMode::Run,
        false,
        None,
        None,
        None,
        3,
        Some(200),
        None,
        None,
        None, // timeout_warning_secs
        None, // timeout_notify_interval_ratio
    )
    .await
    .unwrap();

    // Enter Waiting and start a 1-second timeout.
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    m.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;

    // Wait for timeout to fire.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(
        !m.is_session_yielding(&parent_id).await,
        "session should exit Waiting after timeout"
    );

    // Timeout notification should list both children.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_timeout = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(
                    b,
                    closeclaw_llm::types::ContentBlock::Text(t) if t.contains("等待上限")
                )
            })
    });
    assert!(has_timeout, "timeout notification should be present");
}

// ── 22. No children timeout fires and structured notification ──────────────

/// When no children exist, the structured notification should indicate
/// "(无子 session)" and the session should resume.
#[tokio::test]
#[serial]
async fn test_yield_timeout_no_children_structured_notification() {
    clear_global_prompt_state();

    let m = mgr();
    let parent_id = setup_parent_with_conv(&m, "parent-ncsn").await;

    // Enter Waiting without spawning children.
    {
        let cs = m.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    m.start_yield_timeout(&parent_id, "agent-x", 1, None, None)
        .await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Session should have resumed.
    assert!(!m.is_session_yielding(&parent_id).await);

    // Notification should mention no children.
    let cs = m.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_no_children = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(
                    b,
                    closeclaw_llm::types::ContentBlock::Text(t) if t.contains("无子 session")
                )
            })
    });
    assert!(has_no_children, "notification should mention no children");
}
