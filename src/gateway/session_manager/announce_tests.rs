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
use crate::llm::session::AnnounceEvent;
use crate::llm::types::ContentBlock;
use chrono::Utc;
use serial_test::serial;
use std::collections::HashSet;
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
        )
        .await
        .expect("create_child_session should succeed");

    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![
            ContentBlock::Thinking("secret reasoning that should NOT leak".to_string()),
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
