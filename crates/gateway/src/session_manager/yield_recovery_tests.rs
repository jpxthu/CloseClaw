//! Tests for yield recovery: auto-recovery when all children complete,
//! announce injection, timeout cancellation, and message queue drain.
//!
//! Covers Step 1.6 (Waiting 消息排队与自动恢复) and integration
//! scenarios across Steps 1.5–1.7.

use super::spawn::SpawnMode;
use super::test_helpers::{
    append_assistant_to_child, setup_parent_with_conv, spawn_n_run_children, test_resolved_config,
};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_session::llm_session::ChatSession;
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;

// ── Helper: complete a child and remove it from the SpawnTree ──────────────

async fn complete_and_remove_child(
    mgr: &super::SessionManager,
    child_id: &str,
    parent_id: &str,
    blocks: Vec<closeclaw_llm::types::ContentBlock>,
) {
    append_assistant_to_child(mgr, child_id, blocks).await;
    mgr.try_push_announce(child_id, NotificationPriority::Next)
        .await;
    mgr.children.write().await.remove_child(parent_id, child_id);
}

// ── 1. Single child: recovery after completion + removal ───────────────────

/// When the last run-mode child completes and is removed from the tree,
/// triggering recovery exits Waiting.
#[tokio::test]
#[serial]
async fn test_yield_recovery_single_child() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-recovery").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-r1", None),
            &parent_id,
            1,
            "do task",
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
            None, // prompt_template_prefix
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    assert!(mgr.is_session_yielding(&parent_id).await);

    // Complete child, push announce, and remove from tree.
    complete_and_remove_child(
        &mgr,
        &child_id,
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("done".into())],
    )
    .await;

    // Trigger recovery — now no run-mode children remain.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent should exit Waiting after all children removed"
    );
}

// ── 2. Two children: recovery only after both are removed ──────────────────

#[tokio::test]
#[serial]
async fn test_yield_recovery_two_children() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-multi").await;

    let child_ids = spawn_n_run_children(&mgr, &parent_id, 2).await;

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete and remove first child.
    complete_and_remove_child(
        &mgr,
        &child_ids[0],
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("first".into())],
    )
    .await;

    // Still waiting — one child remaining.
    mgr.trigger_yield_recovery(&parent_id).await;
    assert!(
        mgr.is_session_yielding(&parent_id).await,
        "parent should still be Waiting with one child remaining"
    );

    // Complete and remove second child.
    complete_and_remove_child(
        &mgr,
        &child_ids[1],
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("second".into())],
    )
    .await;

    // Now recovered.
    mgr.trigger_yield_recovery(&parent_id).await;
    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent should exit Waiting after all children removed"
    );
}

// ── 3. Recovery injects announce messages ──────────────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_recovery_injects_announce() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-inject").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-inj", None),
            &parent_id,
            1,
            "compute",
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
            None, // prompt_template_prefix
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete and remove child.
    complete_and_remove_child(
        &mgr,
        &child_id,
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text(
            "result: 42".into(),
        )],
    )
    .await;

    // Trigger recovery.
    mgr.trigger_yield_recovery(&parent_id).await;

    // Check that the announce was injected as a system message.
    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let messages = cs.read().await.messages().to_vec();
    let has_announce = messages.iter().any(|m| {
        m.role == "system"
            && m.content_blocks.iter().any(|b| {
                matches!(b, closeclaw_llm::types::ContentBlock::Text(t) if t.contains("result: 42"))
            })
    });
    assert!(
        has_announce,
        "announce message should be injected into parent's history"
    );
}

// ── 4. No children at yield → immediate recovery ───────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_no_children_immediate_recovery() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-no-children").await;

    // Enter Waiting — no children spawned.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    assert!(mgr.is_session_yielding(&parent_id).await);

    // Trigger recovery — no run-mode children, should recover immediately.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent with no children should exit Waiting immediately"
    );
}

// ── 5. is_session_yielding tracks state correctly ──────────────────────────

#[tokio::test]
#[serial]
async fn test_is_session_yielding_tracks_state() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-state").await;

    assert!(!mgr.is_session_yielding(&parent_id).await);

    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }
    assert!(mgr.is_session_yielding(&parent_id).await);

    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.exit_waiting();
    }
    assert!(!mgr.is_session_yielding(&parent_id).await);
}

// ── 6. Announce queue is drained during recovery ──────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_recovery_drains_announce_queue() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-drain").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-drain", None),
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
            None, // prompt_template_prefix
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete and remove child.
    complete_and_remove_child(
        &mgr,
        &child_id,
        &parent_id,
        vec![closeclaw_llm::types::ContentBlock::Text("output".into())],
    )
    .await;

    // Trigger recovery.
    mgr.trigger_yield_recovery(&parent_id).await;

    // After recovery, the announce queue should be drained.
    let remaining = mgr.drain_announces(&parent_id).await;
    assert!(
        remaining.is_empty(),
        "announce queue should be empty after recovery"
    );
}

// ── 7. Session-mode child doesn't block recovery ──────────────────────────

#[tokio::test]
#[serial]
async fn test_yield_session_mode_no_block() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-sess").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-sess", None),
            &parent_id,
            1,
            "long task",
            true,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None, // prompt_template_prefix
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Complete session-mode child and remove from tree.
    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text("done".into())],
    )
    .await;
    mgr.try_push_announce(&child_id, NotificationPriority::Next)
        .await;
    mgr.children
        .write()
        .await
        .remove_child(&parent_id, &child_id);

    // Trigger recovery — session-mode child doesn't block.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        !mgr.is_session_yielding(&parent_id).await,
        "parent should recover after session-mode child is removed"
    );
}

// ── 8. Recovery does NOT happen while children remain ──────────────────────

/// While a run-mode child is still in the SpawnTree, recovery should
/// not trigger.
#[tokio::test]
#[serial]
async fn test_yield_no_recovery_while_child_registered() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-stay").await;

    let _child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-stay", None),
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
            None, // prompt_template_prefix
        )
        .await
        .unwrap();

    // Enter Waiting.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        cs.read().await.enter_waiting();
    }

    // Trigger recovery — child still in tree.
    mgr.trigger_yield_recovery(&parent_id).await;

    assert!(
        mgr.is_session_yielding(&parent_id).await,
        "parent should remain in Waiting while child is registered"
    );
}
