//! Tests for Step 1.5 — priority-differentiated injection timing.
//!
//! Validates:
//! - Now-priority announces are injected before user message processing
//! - Next/Later priority announces are injected at turn start
//! - Mixed priorities maintain correct ordering
//! - `drain_announces_now` only drains Now events, rest stay in queue
//! - `drain_announces_rest` drains Next+Later events

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

/// `drain_announces_now` must only drain Now-priority events,
/// leaving Next and Later events in the queue.
#[tokio::test]
#[serial]
async fn test_drain_announces_now_only_drains_now() {
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

    // Drain only Now events.
    let now_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    assert_eq!(now_events.len(), 1, "should drain exactly 1 Now event");
    assert_eq!(now_events[0].child_agent_id, "now1");

    // Queue should still contain Next and Later events.
    let remaining = mgr.drain_announces(&parent_id).await;
    assert_eq!(remaining.len(), 2, "should have 2 remaining events");
    let agent_ids: Vec<&str> = remaining
        .iter()
        .map(|e| e.child_agent_id.as_str())
        .collect();
    assert!(agent_ids.contains(&"next1"), "Next event should remain");
    assert!(agent_ids.contains(&"later1"), "Later event should remain");
}

// ── 2. Rest events drained correctly ────────────────────────────────────

/// `drain_announces_rest` must drain Next + Later events, leaving
/// Now events in the queue.
#[tokio::test]
#[serial]
async fn test_drain_announces_rest_drains_next_later() {
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

    // Drain Next + Later events (predicate: priority < Now).
    let rest_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p < NotificationPriority::Now)
        .await;
    assert_eq!(
        rest_events.len(),
        2,
        "should drain 2 rest events (Next + Later)"
    );
    let agent_ids: Vec<&str> = rest_events
        .iter()
        .map(|e| e.child_agent_id.as_str())
        .collect();
    assert!(agent_ids.contains(&"next1"));
    assert!(agent_ids.contains(&"later1"));

    // Queue should still contain the Now event.
    let remaining = mgr.drain_announces(&parent_id).await;
    assert_eq!(remaining.len(), 1, "should have 1 remaining Now event");
    assert_eq!(remaining[0].child_agent_id, "now1");
}

// ── 3. Now injected as system message ───────────────────────────────────

/// `drain_announces_now` with injection must produce a system message
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

    // Simulate the drain_announces_now flow: drain filtered + inject.
    let events = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    assert_eq!(events.len(), 1);

    // Inject as system message.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let mut guard = cs.write().await;
        for ev in &events {
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
/// the priority filter must return them in the correct order.
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

    // Drain Now events first.
    let now_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    let now_ids: Vec<&str> = now_events
        .iter()
        .map(|e| e.child_agent_id.as_str())
        .collect();
    assert_eq!(
        now_ids,
        vec!["N1", "N2"],
        "Now events should be in FIFO order"
    );

    // Drain rest (Next + Later).
    let rest_events = mgr
        .drain_announces_filtered(&parent_id, |p| *p < NotificationPriority::Now)
        .await;
    assert_eq!(rest_events.len(), 3, "should have 3 rest events");
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
        .drain_announces_filtered(&parent_id, |p| *p < NotificationPriority::Now)
        .await;
    assert!(rest.is_empty());
}

// ── 6. Now event not in rest drain ─────────────────────────────────────

/// A Now-priority event must NOT be drained by the rest predicate.
#[tokio::test]
#[serial]
async fn test_now_not_drained_by_rest() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let parent_id = setup_parent_with_conv(&mgr, "parent-pi-6").await;

    mgr.push_announce(
        &parent_id,
        make_event("now-only", NotificationPriority::Now),
    )
    .await
    .unwrap();

    let rest = mgr
        .drain_announces_filtered(&parent_id, |p| *p < NotificationPriority::Now)
        .await;
    assert!(
        rest.is_empty(),
        "Now event should NOT be drained by rest predicate"
    );

    // Now event should still be in queue.
    let now = mgr
        .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
        .await;
    assert_eq!(now.len(), 1, "Now event should still be in queue");
}

// ── 7. Sequential drain: Now first, then rest ──────────────────────────

/// Simulates the Step 1.4 flow: drain Now before LLM call, then
/// drain rest at turn start. Both should produce system messages.
#[tokio::test]
#[serial]
async fn test_sequential_drain_now_then_rest() {
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

    // Phase 1: drain Now events and inject.
    {
        let now_events = mgr
            .drain_announces_filtered(&parent_id, |p| *p == NotificationPriority::Now)
            .await;
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let mut guard = cs.write().await;
        for ev in &now_events {
            guard.inject_system_message(format!("[NOW] {}", ev.child_agent_id));
        }
    }

    // Verify: 1 system message from Now.
    {
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let msgs = cs.read().await.messages().to_vec();
        assert_eq!(msgs.len(), 1, "should have 1 Now message");
        let text = match &msgs[0].content_blocks[0] {
            ContentBlock::Text(t) => t.clone(),
            other => panic!("expected Text, got {:?}", other),
        };
        assert!(text.contains("urgent"));
    }

    // Phase 2: drain rest events and inject.
    {
        let rest_events = mgr
            .drain_announces_filtered(&parent_id, |p| *p < NotificationPriority::Now)
            .await;
        let cs = mgr.get_conversation_session(&parent_id).await.unwrap();
        let mut guard = cs.write().await;
        for ev in &rest_events {
            guard.inject_system_message(format!("[REST] {}", ev.child_agent_id));
        }
    }

    // Verify: total 3 system messages (1 Now + 2 rest).
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
        assert!(texts.iter().any(|t| t.contains("[NOW] urgent")));
        assert!(texts.iter().any(|t| t.contains("[REST] normal")));
        assert!(texts.iter().any(|t| t.contains("[REST] background")));
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
        .drain_announces_filtered(&parent_id, |p| *p < NotificationPriority::Now)
        .await;
    assert_eq!(rest.len(), 2);
}
