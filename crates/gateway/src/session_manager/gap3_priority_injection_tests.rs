//! Tests for priority-differentiated injection (Step 1.3 unification).
//!
//! After Step 1.3, all priorities are drained in a single pass at turn
//! end. Priority only determines queue ordering and display prefix, not
//! injection timing.
//!
//! Validates:
//! - All priority events are drained together at turn end
//! - Mixed priorities maintain correct ordering (Now → Next → Later)
//! - `drain_and_inject_announces_filtered` with `<= Now` drains all
//! - `drain_and_inject_announces_filtered` with `== Now` still works
//!   for targeted filtering at the SessionManager level

use super::test_helpers::setup_parent_with_conv;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use chrono::Utc;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::llm_session::{AnnounceEvent, ChatSession};
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;

// ── helper ──────────────────────────────────────────────────────────────

fn make_event(agent_id: &str, priority: NotificationPriority) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: format!("child_{}", agent_id),
        child_agent_id: agent_id.to_string(),
        result_text: format!("result from {}", agent_id),
        completed_at: Utc::now(),
        priority,
        status: closeclaw_common::ChildCompletionStatus::Completed,
    }
}

// ── 1. Now events drained first ─────────────────────────────────────────

/// All three priorities must be drained in a single pass with
/// the `<= Now` predicate (unified drain, design-doc §通知机制).
#[tokio::test]
#[serial]
async fn test_unified_drain_all_priorities() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-1").await;

    // Push events with all three priorities.
    mgr.push_announce(
        &parent_id,
        make_event("later1", NotificationPriority::Later),
    )
    .await
    .unwrap();
    mgr.push_announce(&parent_id, make_event("now1", NotificationPriority::Now))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("next1", NotificationPriority::Next))
        .await
        .unwrap();

    // Drain all events with the unified predicate.
    let all_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p <= NotificationPriority::Now)
        .await;
    assert_eq!(all_events.len(), 3, "should drain all 3 events");
    let agent_ids: Vec<&str> = all_events
        .iter()
        .map(|e| e.child_agent_id.as_str())
        .collect();
    assert!(agent_ids.contains(&"now1"), "Now event should be drained");
    assert!(agent_ids.contains(&"next1"), "Next event should be drained");
    assert!(
        agent_ids.contains(&"later1"),
        "Later event should be drained"
    );

    // Queue should be empty.
    let remaining = mgr.drain_announces(&parent_id).await;
    assert!(
        remaining.is_empty(),
        "queue should be empty after unified drain"
    );
}

// ── 2. Rest events drained correctly ────────────────────────────────────

/// `drain_announces_rest` (now unified) must drain all priorities,
/// preserving queue insertion order.
#[tokio::test]
#[serial]
async fn test_drain_rest_drains_all() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-2").await;

    mgr.push_announce(&parent_id, make_event("now1", NotificationPriority::Now))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("next1", NotificationPriority::Next))
        .await
        .unwrap();
    mgr.push_announce(
        &parent_id,
        make_event("later1", NotificationPriority::Later),
    )
    .await
    .unwrap();

    // Drain all events (predicate: priority <= Now).
    let all_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p <= NotificationPriority::Now)
        .await;
    assert_eq!(all_events.len(), 3, "should drain all 3 events");
    let agent_ids: Vec<&str> = all_events
        .iter()
        .map(|e| e.child_agent_id.as_str())
        .collect();
    assert!(agent_ids.contains(&"now1"));
    assert!(agent_ids.contains(&"next1"));
    assert!(agent_ids.contains(&"later1"));

    // Queue should be empty.
    let remaining = mgr.drain_announces(&parent_id).await;
    assert!(
        remaining.is_empty(),
        "queue should be empty after unified drain"
    );
}

// ── 3. Now injected as system message ───────────────────────────────────

/// Filtering with `== Now` must produce a system message
/// with the Now event content.
#[tokio::test]
#[serial]
async fn test_now_injected_as_system_message() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-3").await;

    mgr.push_announce(&parent_id, make_event("urgent", NotificationPriority::Now))
        .await
        .unwrap();

    // Simulate a Now-only filter flow: drain filtered + inject.
    let events = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    assert_eq!(events.len(), 1);

    // Inject as system message.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let mut guard = cs.write().await;
        for ev in &events.announces {
            guard.inject_system_message(format!(
                "[子 agent {}] 任务已完成：\n{}",
                ev.child_agent_id, ev.result_text
            ));
        }
    }

    let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
    let msgs = cs.read().await.messages().to_vec();
    assert_eq!(msgs.len(), 1);
    let text = match &msgs[0].content_blocks[0] {
        ContentBlock::Text(t) => t.clone(),
        other => panic!("expected Text, got {:?}", other),
    };
    assert!(text.contains("urgent"));
    assert!(text.contains("result from urgent"));
}

// ── 4. Mixed priority ordering preserved ────────────────────────────────

/// When Now, Next, and Later events are all queued, draining with
/// the unified filter must return them sorted by priority
/// (Now → Next → Later), with FIFO within each level.
#[tokio::test]
#[serial]
async fn test_mixed_priority_filtering_order() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-4").await;

    mgr.push_announce(&parent_id, make_event("L1", NotificationPriority::Later))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("N1", NotificationPriority::Now))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("X1", NotificationPriority::Next))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("L2", NotificationPriority::Later))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("N2", NotificationPriority::Now))
        .await
        .unwrap();

    // Drain all events with unified predicate.
    let all_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p <= NotificationPriority::Now)
        .await;
    let all_ids: Vec<&str> = all_events
        .iter()
        .map(|e| e.child_agent_id.as_str())
        .collect();
    // Priority ordering: Now (N1, N2) → Next (X1) → Later (L1, L2)
    // FIFO within each priority level.
    assert_eq!(
        all_ids,
        vec!["N1", "N2", "X1", "L1", "L2"],
        "events should be sorted by priority (Now > Next > Later), FIFO within level"
    );
}

// ── 5. Empty queue drain ────────────────────────────────────────────────

/// Draining an empty queue returns empty Vec for both Now and rest.
#[tokio::test]
#[serial]
async fn test_drain_empty_queue_all_priorities() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-5").await;

    let now = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    assert!(now.is_empty());

    let rest = mgr
        .drain_announces_filtered(&parent_id, |p| *p <= NotificationPriority::Now)
        .await;
    assert!(rest.is_empty());
}

// ── 6. Now event not in rest drain ─────────────────────────────────────

/// A Now-priority event IS drained by the unified `<= Now` predicate.
/// Priority no longer causes two-phase injection.
#[tokio::test]
#[serial]
async fn test_now_drained_by_unified_predicate() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-6").await;

    mgr.push_announce(
        &parent_id,
        make_event("now-only", NotificationPriority::Now),
    )
    .await
    .unwrap();

    let all = mgr
        .drain_announces_filtered(&parent_id, |p| *p <= NotificationPriority::Now)
        .await;
    assert_eq!(
        all.len(),
        1,
        "Now event should be drained by unified predicate"
    );
    assert_eq!(all[0].child_agent_id, "now-only");

    // Queue should be empty.
    let remaining = mgr.drain_announces(&parent_id).await;
    assert!(remaining.is_empty(), "queue should be empty after drain");
}

// ── 7. Sequential drain: Now first, then rest ──────────────────────────

/// Simulates the unified drain flow: all priorities drained at turn
/// end in a single pass, then injected as system messages.
#[tokio::test]
#[serial]
async fn test_unified_drain_all_at_turn_end() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-7").await;

    mgr.push_announce(&parent_id, make_event("urgent", NotificationPriority::Now))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("normal", NotificationPriority::Next))
        .await
        .unwrap();
    mgr.push_announce(
        &parent_id,
        make_event("background", NotificationPriority::Later),
    )
    .await
    .unwrap();

    // Unified drain: all priorities in a single pass.
    let all_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p <= NotificationPriority::Now)
        .await;
    assert_eq!(all_events.len(), 3, "should drain all 3 events");

    // Inject as system messages.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let mut guard = cs.write().await;
        for ev in &all_events.announces {
            guard.inject_system_message(format!("[{:?}] {}", ev.priority, ev.child_agent_id));
        }
    }

    // Verify: total 3 system messages.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let msgs = cs.read().await.messages().to_vec();
        assert_eq!(msgs.len(), 3, "should have 3 total messages");
        let texts: Vec<String> = msgs
            .iter()
            .map(|m| match &m.content_blocks[0] {
                ContentBlock::Text(t) => t.clone(),
                other => panic!("expected Text, got {:?}", other),
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("urgent")));
        assert!(texts.iter().any(|t| t.contains("normal")));
        assert!(texts.iter().any(|t| t.contains("background")));
    }
}

// ── 8. All events are Now-priority ─────────────────────────────────────

/// When all events are Now priority, rest drain returns empty.
#[tokio::test]
#[serial]
async fn test_all_now_priority_rest_empty() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-8").await;

    mgr.push_announce(&parent_id, make_event("n1", NotificationPriority::Now))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("n2", NotificationPriority::Now))
        .await
        .unwrap();

    let now = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    assert_eq!(now.len(), 2);

    let rest = mgr
        .drain_announces_filtered(&parent_id, |p| *p < NotificationPriority::Now)
        .await;
    assert!(
        rest.is_empty(),
        "rest should be empty when all events are Now"
    );
}

// ── 9. All events are rest priority ────────────────────────────────────

/// When all events are Next/Later priority, Now drain returns empty.
#[tokio::test]
#[serial]
async fn test_all_rest_priority_now_empty() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-9").await;

    mgr.push_announce(&parent_id, make_event("x1", NotificationPriority::Next))
        .await
        .unwrap();
    mgr.push_announce(&parent_id, make_event("l1", NotificationPriority::Later))
        .await
        .unwrap();

    let now = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    assert!(
        now.is_empty(),
        "Now drain should be empty when all events are rest priority"
    );

    let rest = mgr
        .drain_announces_filtered(&parent_id, |p| *p <= NotificationPriority::Now)
        .await;
    assert_eq!(rest.len(), 2);
}
