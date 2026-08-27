//! Step 1.5 — Unit tests for all behavior dimensions of Steps 1.1–1.4.
//!
//! Covers:
//! - Position constraint: summary injected before user message
//! - Spawn_mode checkpoint semantics (residual JSON, defaults, absent)
//! - State transition: child Running → Completed removes from summary
//! - Yield still shows summary but reminder behavior unchanged
//! - Integration: spawn → inject → complete → announce order

use super::super::*;
use closeclaw_common::ChildSessionState;
use std::path::PathBuf;

fn make_session(id: &str) -> ConversationSession {
    ConversationSession::new(id.to_string(), "test-model".into(), PathBuf::from("/tmp"))
}

// ═══════════════════════════════════════════════════════════════════════════
// Position constraint: summary must appear before user message
// ═══════════════════════════════════════════════════════════════════════════

/// When summary is injected before user message, the system message
/// (summary) appears at a lower index than the user message in the
/// transcript.
#[test]
fn test_summary_before_user_message_position() {
    let mut cs = make_session("pos_1");
    cs.register_child("c1", "agent-a", "review code");

    // Inject summary BEFORE user message (simulating dispatch flow).
    let summary = cs.active_children_summary().unwrap();
    cs.inject_system_message(summary);
    cs.append_user_message("hello");

    // Summary (system) must be before user message.
    assert_eq!(cs.messages.len(), 2);
    assert_eq!(cs.messages[0].role, "system");
    match &cs.messages[0].content_blocks[0] {
        ContentBlock::Text(t) => assert!(t.contains("agent-a")),
        _ => panic!("expected Text content block"),
    }
    assert_eq!(cs.messages[1].role, "user");
    assert_eq!(
        cs.messages[1].content_blocks[0],
        ContentBlock::Text("hello".into())
    );
}

/// When there is no active child, no summary is injected — user
/// message is the only message.
#[test]
fn test_no_children_no_summary_before_user() {
    let mut cs = make_session("pos_2");
    cs.append_user_message("hello");

    assert_eq!(cs.messages.len(), 1);
    assert_eq!(cs.messages[0].role, "user");
}

/// With multiple prior messages, the summary + yield reminder still
/// appears before the new user message.
#[test]
fn test_summary_before_user_with_prior_history() {
    let mut cs = make_session("pos_3");
    cs.append_user_message("first question");
    cs.append_transcript("assistant", vec![ContentBlock::Text("first answer".into())]);

    cs.register_child("c1", "agent-a", "task 1");
    let summary = cs.active_children_summary().unwrap();
    let reminder = cs.spawn_guard_reminder().unwrap();
    let mut text = summary;
    text.push('\n');
    text.push_str(&reminder);
    cs.inject_system_message(text);
    cs.append_user_message("second question");

    // Order: user, assistant, system(summary), user
    assert_eq!(cs.messages.len(), 4);
    assert_eq!(cs.messages[0].role, "user");
    assert_eq!(cs.messages[1].role, "assistant");
    assert_eq!(cs.messages[2].role, "system");
    assert_eq!(cs.messages[3].role, "user");
    // System message contains agent info.
    match &cs.messages[2].content_blocks[0] {
        ContentBlock::Text(t) => assert!(t.contains("agent-a")),
        _ => panic!("expected Text content block"),
    }
}

/// Summary + yield reminder combined text appears before user message.
#[test]
fn test_summary_and_reminder_before_user_combined() {
    let mut cs = make_session("pos_4");
    cs.register_child("c1", "agent-a", "task 1");

    let summary = cs.active_children_summary().unwrap();
    let reminder = cs.spawn_guard_reminder().unwrap();
    let mut text = summary;
    text.push('\n');
    text.push_str(&reminder);
    cs.inject_system_message(text);
    cs.append_user_message("go");

    assert_eq!(cs.messages.len(), 2);
    assert_eq!(cs.messages[0].role, "system");
    match &cs.messages[0].content_blocks[0] {
        ContentBlock::Text(t) => {
            assert!(t.contains("agent-a"));
            assert!(t.contains("yield"));
        }
        _ => panic!("expected Text content block"),
    }
    assert_eq!(cs.messages[1].role, "user");
}

// ═══════════════════════════════════════════════════════════════════════════
// Spawn_mode checkpoint semantics
// ═══════════════════════════════════════════════════════════════════════════

/// New checkpoint does not contain "spawn_mode" field in serialized JSON.
#[test]
fn test_new_checkpoint_no_spawn_mode_field() {
    use crate::persistence::SessionCheckpoint;

    let cp = SessionCheckpoint::new("test-session".to_string());
    let json = serde_json::to_value(&cp).unwrap();
    assert!(
        !json.as_object().unwrap().contains_key("spawn_mode"),
        "new checkpoint must not have spawn_mode field"
    );
}

/// Old checkpoint JSON with residual "spawn_mode" key deserializes
/// without error (serde ignores unknown fields).
#[test]
fn test_old_checkpoint_with_spawn_mode_key_deserializes() {
    use crate::persistence::SessionCheckpoint;

    let raw = serde_json::json!({
        "session_id": "old-session",
        "spawn_mode": "run",
        "depth": 1,
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z",
        "ttl_seconds": 604800,
        "status": "active",
        "reasoning_mode": "direct",
        "reasoning_level": "low",
        "dreaming_status": "completed",
        "message_count": 0,
        "session_mode": "normal",
        "outbound_pending": [],
        "system_appends": [],
        "pending_operations": [],
        "pending_tool_failures": [],
        "progress_tool_calls": [],
        "approval_tool_calls": [],
        "plan_references": [],
        "pending_messages": [],
        "snapshot_metas": [],
        "mined": false,
        "mode_state": {
          "current_step": 0,
          "total_steps": 0,
          "step_messages": [],
          "is_complete": false
        },
        "verbosity_level": "full"
    });

    // Must not error — unknown "spawn_mode" key is silently ignored.
    let cp: SessionCheckpoint = serde_json::from_value(raw).unwrap();
    assert_eq!(cp.session_id, "old-session");
    assert_eq!(cp.depth, 1);
}

/// Serialized new checkpoint roundtrips without spawn_mode.
#[test]
fn test_checkpoint_serialization_roundtrip_no_spawn_mode() {
    use crate::persistence::SessionCheckpoint;

    let cp = SessionCheckpoint::new("rt-session".to_string());
    let json_str = serde_json::to_string(&cp).unwrap();
    assert!(
        !json_str.contains("spawn_mode"),
        "serialized checkpoint must not contain spawn_mode"
    );
    let restored: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
    assert_eq!(restored.session_id, "rt-session");
}

// ═══════════════════════════════════════════════════════════════════════════
// State transition: child Running → Completed removes from summary
// ═══════════════════════════════════════════════════════════════════════════

/// After child transitions from Running to Completed, the summary
/// no longer includes that child.
#[test]
fn test_running_to_completed_removes_from_summary() {
    let cs = make_session("st_1");
    cs.register_child("c1", "agent-a", "task A");
    cs.register_child("c2", "agent-b", "task B");

    // Both active.
    let summary = cs.active_children_summary().unwrap();
    assert!(summary.contains("agent-a"));
    assert!(summary.contains("agent-b"));

    // Complete one.
    cs.update_child_state("c1", ChildSessionState::Completed);
    let summary = cs.active_children_summary().unwrap();
    assert!(!summary.contains("agent-a"));
    assert!(summary.contains("agent-b"));

    // Complete the other.
    cs.update_child_state("c2", ChildSessionState::Completed);
    assert!(cs.active_children_summary().is_none());
}

/// Child terminated → not in summary.
#[test]
fn test_terminated_child_not_in_summary() {
    let cs = make_session("st_2");
    cs.register_child("c1", "agent-a", "task");
    cs.update_child_state("c1", ChildSessionState::Terminated);
    assert!(cs.active_children_summary().is_none());
}

/// Yield (enter_waiting) does not remove children from summary.
#[test]
fn test_yield_still_shows_summary() {
    let cs = make_session("st_3");
    cs.register_child("c1", "agent-a", "task");
    cs.enter_waiting();

    // Summary still shows the running child.
    let summary = cs.active_children_summary();
    assert!(summary.is_some());
    assert!(summary.unwrap().contains("agent-a"));
}

/// After yield, spawn_guard_reminder returns None (already yielded),
/// but summary is still present.
#[test]
fn test_yield_reminder_none_but_summary_present() {
    let cs = make_session("st_4");
    cs.register_child("c1", "agent-a", "task");
    cs.enter_waiting();

    // Reminder: None (already yielded).
    assert!(cs.spawn_guard_reminder().is_none());
    // Summary: still present.
    assert!(cs.active_children_summary().is_some());
}

// ═══════════════════════════════════════════════════════════════════════════
// Boundary values
// ═══════════════════════════════════════════════════════════════════════════

/// No children → summary is None, no injection.
#[test]
fn test_no_children_summary_none() {
    let cs = make_session("bv_1");
    assert!(cs.active_children_summary().is_none());
    assert!(cs.spawn_guard_reminder().is_none());
}

/// All children completed → summary is None.
#[test]
fn test_all_completed_summary_none() {
    let cs = make_session("bv_2");
    cs.register_child("c1", "agent-a", "task");
    cs.update_child_state("c1", ChildSessionState::Completed);
    assert!(cs.active_children_summary().is_none());
}

/// All children terminated → summary is None.
#[test]
fn test_all_terminated_summary_none() {
    let cs = make_session("bv_3");
    cs.register_child("c1", "agent-a", "task");
    cs.update_child_state("c1", ChildSessionState::Terminated);
    assert!(cs.active_children_summary().is_none());
}

/// Running child with detail=None (defensive) is skipped in summary.
#[test]
fn test_none_detail_running_child_skipped() {
    let cs = make_session("bv_4");
    {
        let mut states = cs.child_states.write().unwrap();
        states.insert(
            "c_no_detail".to_string(),
            (ChildSessionState::Running, None),
        );
        states.insert(
            "c_with_detail".to_string(),
            (
                ChildSessionState::Running,
                Some(PendingOperationDetail::SubSessionSpawn {
                    child_session_id: "c_with_detail".to_string(),
                    agent_id: "agent-x".to_string(),
                    task_summary: "real task".to_string(),
                }),
            ),
        );
    }
    let summary = cs.active_children_summary().unwrap();
    assert!(summary.contains("agent-x"));
    assert!(summary.contains("real task"));
    // Only 1 item in list.
    assert_eq!(summary.matches('\n').count(), 1);
}

/// Mixed states: only Running children appear in summary.
#[test]
fn test_mixed_states_only_running_in_summary() {
    let cs = make_session("bv_5");
    cs.register_child("c1", "agent-a", "running task");
    cs.register_child("c2", "agent-b", "completed task");
    cs.register_child("c3", "agent-c", "terminated task");
    cs.update_child_state("c2", ChildSessionState::Completed);
    cs.update_child_state("c3", ChildSessionState::Terminated);

    let summary = cs.active_children_summary().unwrap();
    assert!(summary.contains("agent-a"));
    assert!(!summary.contains("agent-b"));
    assert!(!summary.contains("agent-c"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Integration: spawn → inject → complete → announce order
// ═══════════════════════════════════════════════════════════════════════════

/// Simulates the full lifecycle:
/// 1. Parent registers child (spawn)
/// 2. Summary injected before user message (dispatch)
/// 3. Child completes
/// 4. Announce injected (announce)
///
/// Verifies the transcript order is correct.
#[test]
fn test_integration_spawn_inject_complete_announce() {
    let mut cs = make_session("int_1");

    // Step 1: Spawn child.
    cs.register_child("child-1", "agent-a", "review PR");

    // Step 2: Inject summary before user message (dispatch).
    let summary = cs.active_children_summary().unwrap();
    let reminder = cs.spawn_guard_reminder().unwrap();
    let mut text = summary;
    text.push('\n');
    text.push_str(&reminder);
    cs.inject_system_message(text);
    cs.append_user_message("check status");

    // Transcript: [system(summary), user]
    assert_eq!(cs.messages.len(), 2);
    assert_eq!(cs.messages[0].role, "system");
    assert_eq!(cs.messages[1].role, "user");

    // Step 3: Child completes.
    cs.update_child_state("child-1", ChildSessionState::Completed);

    // Step 4: Announce injected.
    cs.inject_system_message("child-1 completed: PR review done".to_string());

    // Transcript: [system(summary), user, system(announce)]
    assert_eq!(cs.messages.len(), 3);
    assert_eq!(cs.messages[0].role, "system");
    assert_eq!(cs.messages[1].role, "user");
    assert_eq!(cs.messages[2].role, "system");
    match &cs.messages[2].content_blocks[0] {
        ContentBlock::Text(t) => assert!(
            t.contains("completed"),
            "announce message should contain 'completed'"
        ),
        _ => panic!("expected Text content block"),
    }
}

/// After child completes, a new dispatch no longer injects summary
/// for that child (only the announce is present).
#[test]
fn test_integration_completed_child_no_summary_in_next_dispatch() {
    let mut cs = make_session("int_2");

    // Spawn and complete child.
    cs.register_child("child-1", "agent-a", "task");
    cs.update_child_state("child-1", ChildSessionState::Completed);

    // Next dispatch: no summary injection.
    assert!(cs.active_children_summary().is_none());
    assert!(cs.spawn_guard_reminder().is_none());

    // User message is the only thing appended.
    cs.append_user_message("next question");
    assert_eq!(cs.messages.len(), 1);
    assert_eq!(cs.messages[0].role, "user");
}

/// Multiple children: some complete, summary only shows remaining.
#[test]
fn test_integration_partial_complete_summary_correct() {
    let mut cs = make_session("int_3");

    cs.register_child("c1", "agent-a", "task A");
    cs.register_child("c2", "agent-b", "task B");

    // Inject summary with both children.
    let summary = cs.active_children_summary().unwrap();
    assert!(summary.contains("agent-a"));
    assert!(summary.contains("agent-b"));
    cs.inject_system_message(summary);
    cs.append_user_message("status?");

    // c1 completes.
    cs.update_child_state("c1", ChildSessionState::Completed);

    // Next dispatch: only c2 in summary.
    let summary = cs.active_children_summary().unwrap();
    assert!(!summary.contains("agent-a"));
    assert!(summary.contains("agent-b"));
}

/// Yield + summary: after yielding, summary still injected but
/// reminder is None (no yield suggestion).
#[test]
fn test_integration_yield_summary_present_reminder_absent() {
    let cs = make_session("int_4");
    cs.register_child("c1", "agent-a", "task");
    cs.enter_waiting();

    let summary = cs.active_children_summary();
    let reminder = cs.spawn_guard_reminder();
    assert!(summary.is_some());
    assert!(reminder.is_none());
}

/// Concurrent children: all show in summary, then complete one by one.
#[test]
fn test_integration_concurrent_children_summary_count() {
    let cs = make_session("int_5");
    cs.register_child("c1", "agent-a", "task 1");
    cs.register_child("c2", "agent-b", "task 2");
    cs.register_child("c3", "agent-c", "task 3");

    let summary = cs.active_children_summary().unwrap();
    // 3 items in list = 3 newlines (header + 3 items).
    assert_eq!(summary.matches('\n').count(), 3);
    assert!(summary.contains("agent-a"));
    assert!(summary.contains("agent-b"));
    assert!(summary.contains("agent-c"));

    cs.update_child_state("c1", ChildSessionState::Completed);
    let summary = cs.active_children_summary().unwrap();
    assert_eq!(summary.matches('\n').count(), 2);

    cs.update_child_state("c2", ChildSessionState::Completed);
    let summary = cs.active_children_summary().unwrap();
    assert_eq!(summary.matches('\n').count(), 1);

    cs.update_child_state("c3", ChildSessionState::Completed);
    assert!(cs.active_children_summary().is_none());
}
