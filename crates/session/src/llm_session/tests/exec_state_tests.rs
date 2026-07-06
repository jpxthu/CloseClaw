//! Tests for the three-dimensional execution state model.
//!
//! Covers LlmState, ToolExecState, ChildSessionState, exec_status(),
//! and is_llm_busy() delegation.

use super::super::*;
use closeclaw_common::{ChildSessionState, LlmState, SessionExecStatus, ToolExecState};
use std::sync::Arc;
use std::thread;

// ── LlmState ──────────────────────────────────────────────────────────────

#[test]
fn test_llm_state_default_idle() {
    let session = ConversationSession::new("s_llm_1".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.llm_state(), LlmState::Idle);
}

#[test]
fn test_set_llm_state_requesting() {
    let session = ConversationSession::new("s_llm_2".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Requesting);
    assert_eq!(session.llm_state(), LlmState::Requesting);
}

#[test]
fn test_set_llm_state_receiving() {
    let session = ConversationSession::new("s_llm_3".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Receiving);
    assert_eq!(session.llm_state(), LlmState::Receiving);
}

#[test]
fn test_set_llm_state_cycle() {
    let session = ConversationSession::new("s_llm_4".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.llm_state(), LlmState::Idle);
    session.set_llm_state(LlmState::Requesting);
    assert_eq!(session.llm_state(), LlmState::Requesting);
    session.set_llm_state(LlmState::Receiving);
    assert_eq!(session.llm_state(), LlmState::Receiving);
    session.set_llm_state(LlmState::Idle);
    assert_eq!(session.llm_state(), LlmState::Idle);
}

// ── is_llm_busy delegates to exec_status ──────────────────────────────────

#[test]
fn test_is_llm_busy_default_false() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into(), tmp_path());
    assert!(!session.is_llm_busy());
}

#[test]
fn test_is_llm_busy_true_when_requesting() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Requesting);
    assert!(session.is_llm_busy());
}

#[test]
fn test_is_llm_busy_true_when_receiving() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Receiving);
    assert!(session.is_llm_busy());
}

#[test]
fn test_is_llm_busy_false_when_idle() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Requesting);
    assert!(session.is_llm_busy());
    session.set_llm_state(LlmState::Idle);
    assert!(!session.is_llm_busy());
}

#[test]
fn test_is_llm_busy_false_with_background_tool_only() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("bg_1");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    assert!(!session.is_llm_busy());
}

// ── ToolExecState ─────────────────────────────────────────────────────────

#[test]
fn test_register_tool_call_new() {
    let session = ConversationSession::new("s_tool_1".into(), "gpt-4o".into(), tmp_path());
    assert!(session.register_tool_call("call_1"));
    assert!(!session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());
}

#[test]
fn test_register_tool_call_duplicate() {
    let session = ConversationSession::new("s_tool_2".into(), "gpt-4o".into(), tmp_path());
    assert!(session.register_tool_call("call_1"));
    assert!(!session.register_tool_call("call_1"));
}

#[test]
fn test_update_tool_state_foreground() {
    let session = ConversationSession::new("s_tool_3".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert!(session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());
}

#[test]
fn test_update_tool_state_background() {
    let session = ConversationSession::new("s_tool_4".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1");
    session.update_tool_state("call_1", ToolExecState::RunningBackground);
    assert!(!session.has_active_foreground_tool());
    assert!(session.has_active_background_tool());
}

#[test]
fn test_update_tool_state_unknown_id_no_panic() {
    let session = ConversationSession::new("s_tool_5".into(), "gpt-4o".into(), tmp_path());
    session.update_tool_state("nonexistent", ToolExecState::Completed);
}

#[test]
fn test_deregister_tool_call() {
    let session = ConversationSession::new("s_tool_6".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert!(session.has_active_foreground_tool());
    session.deregister_tool_call("call_1");
    assert!(!session.has_active_foreground_tool());
}

#[test]
fn test_deregister_tool_call_unknown_id_no_panic() {
    let session = ConversationSession::new("s_tool_7".into(), "gpt-4o".into(), tmp_path());
    session.deregister_tool_call("nonexistent");
}

#[test]
fn test_tool_lifecycle_full() {
    let session = ConversationSession::new("s_tool_8".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert!(session.has_active_foreground_tool());
    session.update_tool_state("call_1", ToolExecState::Completed);
    assert!(!session.has_active_foreground_tool());
    session.deregister_tool_call("call_1");
}

// ── ChildSessionState ─────────────────────────────────────────────────────

#[test]
fn test_register_child_new() {
    let session = ConversationSession::new("s_child_1".into(), "gpt-4o".into(), tmp_path());
    assert!(session.register_child("child_1"));
    assert!(session.has_running_child());
}

#[test]
fn test_register_child_duplicate() {
    let session = ConversationSession::new("s_child_2".into(), "gpt-4o".into(), tmp_path());
    assert!(session.register_child("child_1"));
    assert!(!session.register_child("child_1"));
}

#[test]
fn test_update_child_state() {
    let session = ConversationSession::new("s_child_3".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1");
    session.update_child_state("child_1", ChildSessionState::Completed);
    assert!(!session.has_running_child());
}

#[test]
fn test_update_child_state_unknown_id_no_panic() {
    let session = ConversationSession::new("s_child_4".into(), "gpt-4o".into(), tmp_path());
    session.update_child_state("nonexistent", ChildSessionState::Completed);
}

#[test]
fn test_deregister_child() {
    let session = ConversationSession::new("s_child_5".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1");
    assert!(session.has_running_child());
    session.deregister_child("child_1");
    assert!(!session.has_running_child());
}

#[test]
fn test_deregister_child_unknown_id_no_panic() {
    let session = ConversationSession::new("s_child_6".into(), "gpt-4o".into(), tmp_path());
    session.deregister_child("nonexistent");
}

// ── exec_status() — state table coverage ──────────────────────────────────

#[test]
fn test_exec_status_idle() {
    let session = ConversationSession::new("s_exec_1".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

#[test]
fn test_exec_status_busy_llm_requesting() {
    let session = ConversationSession::new("s_exec_2".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Requesting);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

#[test]
fn test_exec_status_busy_llm_receiving() {
    let session = ConversationSession::new("s_exec_3".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Receiving);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

#[test]
fn test_exec_status_busy_foreground_tool() {
    let session = ConversationSession::new("s_exec_4".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

#[test]
fn test_exec_status_waiting_child_running() {
    let session = ConversationSession::new("s_exec_5".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1");
    assert_eq!(session.exec_status(), SessionExecStatus::Waiting);
}

#[test]
fn test_exec_status_idle_with_background_tasks() {
    let session = ConversationSession::new("s_exec_6".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("bg_1");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    assert_eq!(
        session.exec_status(),
        SessionExecStatus::IdleWithBackgroundTasks
    );
}

#[test]
fn test_exec_status_busy_llm_overrides_background_tool() {
    let session = ConversationSession::new("s_exec_7".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("bg_1");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    session.set_llm_state(LlmState::Requesting);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

#[test]
fn test_exec_status_busy_foreground_overrides_waiting() {
    let session = ConversationSession::new("s_exec_8".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1");
    session.register_tool_call("call_1");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// ── Concurrent register/deregister ────────────────────────────────────────

#[test]
fn test_concurrent_tool_register_deregister_no_panic() {
    let session = Arc::new(ConversationSession::new(
        "s_conc_tool".into(),
        "gpt-4o".into(),
        tmp_path(),
    ));
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let s = Arc::clone(&session);
            thread::spawn(move || {
                let id = format!("call_{}", i);
                s.register_tool_call(&id);
                s.update_tool_state(&id, ToolExecState::RunningForeground);
                s.deregister_tool_call(&id);
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

#[test]
fn test_concurrent_child_register_deregister_no_panic() {
    let session = Arc::new(ConversationSession::new(
        "s_conc_child".into(),
        "gpt-4o".into(),
        tmp_path(),
    ));
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let s = Arc::clone(&session);
            thread::spawn(move || {
                let id = format!("child_{}", i);
                s.register_child(&id);
                s.update_child_state(&id, ChildSessionState::Completed);
                s.deregister_child(&id);
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}
