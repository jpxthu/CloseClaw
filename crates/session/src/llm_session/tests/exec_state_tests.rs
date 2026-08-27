//! Tests for the four-dimensional execution state model.
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
    session.register_tool_call("bg_1", "bash", "ls");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    assert!(!session.is_llm_busy());
}

// ── ToolExecState ─────────────────────────────────────────────────────────

#[test]
fn test_register_tool_call_new() {
    let session = ConversationSession::new("s_tool_1".into(), "gpt-4o".into(), tmp_path());
    assert!(session.register_tool_call("call_1", "bash", "echo test"));
    // Pending tools are treated as foreground-active.
    assert!(session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());
}

#[test]
fn test_register_tool_call_duplicate() {
    let session = ConversationSession::new("s_tool_2".into(), "gpt-4o".into(), tmp_path());
    assert!(session.register_tool_call("call_1", "bash", "echo test"));
    assert!(!session.register_tool_call("call_1", "bash", "echo test"));
}

#[test]
fn test_update_tool_state_foreground() {
    let session = ConversationSession::new("s_tool_3".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1", "bash", "echo");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert!(session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());
}

#[test]
fn test_update_tool_state_background() {
    let session = ConversationSession::new("s_tool_4".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1", "bash", "echo");
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
    session.register_tool_call("call_1", "bash", "echo");
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
    session.register_tool_call("call_1", "bash", "echo");
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
    assert!(session.register_child("child_1", "agent-a", "do something"));
    assert!(session.has_running_child());
}

#[test]
fn test_register_child_duplicate() {
    let session = ConversationSession::new("s_child_2".into(), "gpt-4o".into(), tmp_path());
    assert!(session.register_child("child_1", "agent-a", "do something"));
    assert!(!session.register_child("child_1", "agent-a", "do something"));
}

#[test]
fn test_update_child_state() {
    let session = ConversationSession::new("s_child_3".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "do something");
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
    session.register_child("child_1", "agent-a", "do something");
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
    session.register_tool_call("call_1", "bash", "echo");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

#[test]
fn test_exec_status_busy_pending_tool() {
    let session = ConversationSession::new("s_exec_9".into(), "gpt-4o".into(), tmp_path());
    // A newly registered tool is in Pending state — should still cause Busy.
    session.register_tool_call("call_1", "bash", "echo");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// Per design doc: child_active does NOT affect idle/Busy determination.
// When only child is running (no yielding, no LLM, no foreground tool),
// exec_status should return Idle — not Waiting.
#[test]
fn test_exec_status_idle_child_running_no_yield() {
    let session = ConversationSession::new("s_exec_5".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "do something");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

// Waiting is only returned when is_yielding=true (agent called sessions_yield).
#[test]
fn test_exec_status_waiting_only_when_yielding() {
    let session = ConversationSession::new("s_exec_5y".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "do something");
    // Without yielding, should be Idle.
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
    // With yielding, should be Waiting.
    session.enter_waiting();
    assert_eq!(session.exec_status(), SessionExecStatus::Waiting);
}

#[test]
fn test_exec_status_idle_with_background_tasks() {
    let session = ConversationSession::new("s_exec_6".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("bg_1", "bash", "ls");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    assert_eq!(
        session.exec_status(),
        SessionExecStatus::IdleWithBackgroundTasks
    );
}

#[test]
fn test_exec_status_busy_llm_overrides_background_tool() {
    let session = ConversationSession::new("s_exec_7".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("bg_1", "bash", "ls");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    session.set_llm_state(LlmState::Requesting);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

#[test]
fn test_exec_status_busy_foreground_overrides_waiting() {
    let session = ConversationSession::new("s_exec_8".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "do something");
    session.register_tool_call("call_1", "bash", "echo");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// ── Step 1.6: Complete state-machine lifecycle tests ───────────────────

// 1. 正常路径 — 前台成功: Pending → RunningForeground → Completed → deregister
#[test]
fn test_fg_success_lifecycle_exec_status() {
    let session = ConversationSession::new("s_fg_ok".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);

    // Pending
    session.register_tool_call("call-ok", "bash", "echo hi");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
    assert!(session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());

    // RunningForeground
    session.update_tool_state("call-ok", ToolExecState::RunningForeground);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
    assert!(session.has_active_foreground_tool());

    // Completed
    session.update_tool_state("call-ok", ToolExecState::Completed);
    assert!(!session.has_active_foreground_tool());

    // deregister → Idle
    session.deregister_tool_call("call-ok");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

// 2. 正常路径 — 前台失败（命令非零退出码）: Completed（非 Failed）
#[test]
fn test_fg_command_error_is_completed_not_failed() {
    let session = ConversationSession::new("s_fg_err".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call-err", "bash", "exit 1");
    session.update_tool_state("call-err", ToolExecState::RunningForeground);
    // Command error (non-zero exit) is mapped to Completed, not Failed.
    session.update_tool_state("call-err", ToolExecState::Completed);
    assert!(!session.has_active_foreground_tool());
    session.deregister_tool_call("call-err");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

// 3. 正常路径 — 后台: Pending → RunningBackground → retained → Completed → deregister
#[test]
fn test_bg_lifecycle_exec_status() {
    let session = ConversationSession::new("s_bg_ok".into(), "gpt-4o".into(), tmp_path());
    // Pending → should be treated as foreground-active → Busy
    session.register_tool_call("bg-1", "bash", "ls");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);

    // RunningBackground → no foreground tools → IdleWithBackgroundTasks
    session.update_tool_state("bg-1", ToolExecState::RunningBackground);
    assert_eq!(
        session.exec_status(),
        SessionExecStatus::IdleWithBackgroundTasks
    );
    assert!(!session.has_active_foreground_tool());
    assert!(session.has_active_background_tool());
    // Still in tool_states (not deregistered)
    assert!(session
        .tool_states
        .read()
        .expect("lock")
        .contains_key("bg-1"));

    // Completed → deregister
    session.update_tool_state("bg-1", ToolExecState::Completed);
    session.deregister_tool_call("bg-1");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
    assert!(!session.has_active_background_tool());
}

// 4. 错误路径 — spawn 失败: Pending → Failed → deregister
#[test]
fn test_spawn_failure_lifecycle() {
    let session = ConversationSession::new("s_spawn_f".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("sf-1", "bash", "broken");
    // Pending
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
    assert!(session.has_active_foreground_tool());

    // Failed (spawn error)
    session.update_tool_state("sf-1", ToolExecState::Failed);
    assert!(!session.has_active_foreground_tool());

    // deregister
    session.deregister_tool_call("sf-1");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

// 5. 超时路径: Pending → RunningForeground → RunningBackground（转后台）
#[test]
fn test_fg_timeout_to_bg_preserves_exec_status() {
    let session = ConversationSession::new("s_fg_to".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("to-1", "bash", "sleep 100");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);

    session.update_tool_state("to-1", ToolExecState::RunningForeground);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);

    // Timeout: auto-promote to background
    session.update_tool_state("to-1", ToolExecState::RunningBackground);
    assert_eq!(
        session.exec_status(),
        SessionExecStatus::IdleWithBackgroundTasks
    );
    assert!(!session.has_active_foreground_tool());
    assert!(session.has_active_background_tool());
    // Must remain in tool_states
    assert!(session
        .tool_states
        .read()
        .expect("lock")
        .contains_key("to-1"));
}

// 6. 状态转换边界 — Pending 工具使 exec_status 为 Busy
#[test]
fn test_pending_causes_busy() {
    let session = ConversationSession::new("s_pend".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("pend-1", "bash", "echo");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
    assert!(session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());
}

// 6b. 所有工具注销后 exec_status 返回 Idle
#[test]
fn test_all_deregistered_returns_idle() {
    let session = ConversationSession::new("s_del".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("d-1", "bash", "a");
    session.update_tool_state("d-1", ToolExecState::RunningForeground);
    session.register_tool_call("d-2", "bash", "b");
    session.update_tool_state("d-2", ToolExecState::RunningBackground);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);

    session.deregister_tool_call("d-1");
    session.deregister_tool_call("d-2");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
    assert!(!session.has_active_foreground_tool());
    assert!(!session.has_active_background_tool());
}

// 6c. 多工具混合（前台 + 后台）时 exec_status 返回 Busy
#[test]
fn test_mixed_fg_bg_returns_busy() {
    let session = ConversationSession::new("s_mix".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("mix-fg", "bash", "fg");
    session.update_tool_state("mix-fg", ToolExecState::RunningForeground);
    session.register_tool_call("mix-bg", "bash", "bg");
    session.update_tool_state("mix-bg", ToolExecState::RunningBackground);
    // Busy because foreground tool overrides background tool
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
    assert!(session.has_active_foreground_tool());
    assert!(session.has_active_background_tool());
}

// 6d. Terminated 状态的工具不计入前台活跃
#[test]
fn test_terminated_not_active_foreground() {
    let session = ConversationSession::new("s_term".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("t-1", "bash", "cmd");
    session.update_tool_state("t-1", ToolExecState::RunningForeground);
    assert!(session.has_active_foreground_tool());
    session.update_tool_state("t-1", ToolExecState::Terminated);
    assert!(!session.has_active_foreground_tool());
}

// 6e. collect_pending_operations 收集所有活跃状态的工具
#[test]
fn test_collect_pending_includes_all_active_states() {
    let session = ConversationSession::new("s_cp".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("cp-pend", "bash", "pending cmd");
    session.register_tool_call("cp-fg", "bash", "fg cmd");
    session.update_tool_state("cp-fg", ToolExecState::RunningForeground);
    session.register_tool_call("cp-bg", "bash", "bg cmd");
    session.update_tool_state("cp-bg", ToolExecState::RunningBackground);
    let pending = session.collect_pending_operations();
    let ids: Vec<&str> = pending.iter().map(|op| op.op_id.as_str()).collect();
    assert!(ids.contains(&"cp-pend"));
    assert!(ids.contains(&"cp-fg"));
    assert!(ids.contains(&"cp-bg"));
}

// 6f. collect_pending_operations 不收集已注销的工具
#[test]
fn test_collect_pending_excludes_deregistered() {
    let session = ConversationSession::new("s_cpd".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("cpd-1", "bash", "cmd");
    session.update_tool_state("cpd-1", ToolExecState::Completed);
    session.deregister_tool_call("cpd-1");
    let pending = session.collect_pending_operations();
    assert!(pending.iter().all(|op| op.op_id != "cpd-1"));
}

// 6g. collect_pending_operations 不收集终态工具
#[test]
fn test_collect_pending_excludes_terminal_states() {
    let session = ConversationSession::new("s_cpt".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("cpt-f", "bash", "cmd");
    session.update_tool_state("cpt-f", ToolExecState::Failed);
    session.register_tool_call("cpt-t", "bash", "cmd");
    session.update_tool_state("cpt-t", ToolExecState::Terminated);
    let pending = session.collect_pending_operations();
    assert!(pending.iter().all(|op| op.op_id != "cpt-f"));
    assert!(pending.iter().all(|op| op.op_id != "cpt-t"));
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
                s.register_tool_call(&id, "bash", "cmd");
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
                s.register_child(&id, "agent-x", "task");
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

// ── spawn_guard_reminder (first-layer defense) ────────────────────────────

#[test]
fn test_spawn_guard_reminder_active_children_not_yielded() {
    let session = ConversationSession::new("s_sg1".into(), "gpt-4o".into(), tmp_path());
    // Register two running children.
    session.register_child("child_1", "agent-a", "task 1");
    session.register_child("child_2", "agent-b", "task 2");
    // Not in Waiting state (not yielded).
    assert!(!session.is_waiting());
    // Should return a reminder with count = 2.
    let reminder = session.spawn_guard_reminder();
    assert!(reminder.is_some());
    let msg = reminder.unwrap();
    assert!(
        msg.contains("2"),
        "reminder should mention 2 active children"
    );
    assert!(msg.contains("yield"), "reminder should suggest yielding");
}

#[test]
fn test_spawn_guard_reminder_active_children_yielded() {
    let session = ConversationSession::new("s_sg2".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "task");
    // Enter Waiting state (yielded).
    session.enter_waiting();
    assert!(session.is_waiting());
    // Should return None because session already yielded.
    assert!(session.spawn_guard_reminder().is_none());
}

#[test]
fn test_spawn_guard_reminder_no_children() {
    let session = ConversationSession::new("s_sg3".into(), "gpt-4o".into(), tmp_path());
    // No children registered.
    assert!(!session.has_active_children());
    // Should return None.
    assert!(session.spawn_guard_reminder().is_none());
}

#[test]
fn test_spawn_guard_reminder_all_children_completed() {
    let session = ConversationSession::new("s_sg4".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "task");
    session.update_child_state("child_1", ChildSessionState::Completed);
    assert!(!session.has_active_children());
    // No active children → no reminder.
    assert!(session.spawn_guard_reminder().is_none());
}

#[test]
fn test_spawn_guard_reminder_message_content_format() {
    let session = ConversationSession::new("s_sg5".into(), "gpt-4o".into(), tmp_path());
    // Register 3 running children.
    session.register_child("c1", "agent-a", "task 1");
    session.register_child("c2", "agent-b", "task 2");
    session.register_child("c3", "agent-c", "task 3");
    let reminder = session.spawn_guard_reminder().unwrap();
    assert!(
        reminder.contains("3"),
        "reminder should contain the count of active children"
    );
    // Verify the Chinese message format matches the design doc.
    assert!(
        reminder.starts_with("你有"),
        "reminder should start with the expected prefix"
    );
    assert!(
        reminder.contains("子 agent 仍在运行"),
        "reminder should mention sub-agents running"
    );
    assert!(
        reminder.contains("建议 yield 等待结果"),
        "reminder should suggest yield"
    );
}

// ── active_children_summary ──────────────────────────────────────────────

#[test]
fn test_active_children_summary_returns_none_when_no_children() {
    let session = ConversationSession::new("s_acs1".into(), "gpt-4o".into(), tmp_path());
    assert!(session.active_children_summary().is_none());
}

#[test]
fn test_active_children_summary_returns_none_when_all_completed() {
    let session = ConversationSession::new("s_acs2".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "task 1");
    session.update_child_state("c1", ChildSessionState::Completed);
    assert!(session.active_children_summary().is_none());
}

#[test]
fn test_active_children_summary_returns_none_when_all_terminated() {
    let session = ConversationSession::new("s_acs3".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "task 1");
    session.update_child_state("c1", ChildSessionState::Terminated);
    assert!(session.active_children_summary().is_none());
}

#[test]
fn test_active_children_summary_single_child() {
    let session = ConversationSession::new("s_acs4".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "review code");
    let summary = session.active_children_summary().unwrap();
    assert!(summary.contains("agent-a"));
    assert!(summary.contains("review code"));
    assert!(summary.contains("当前活跃子 Session"));
}

#[test]
fn test_active_children_summary_multiple_children() {
    let session = ConversationSession::new("s_acs5".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "task 1");
    session.register_child("c2", "agent-b", "task 2");
    session.register_child("c3", "agent-c", "task 3");
    let summary = session.active_children_summary().unwrap();
    assert!(summary.contains("agent-a"));
    assert!(summary.contains("task 1"));
    assert!(summary.contains("agent-b"));
    assert!(summary.contains("task 2"));
    assert!(summary.contains("agent-c"));
    assert!(summary.contains("task 3"));
    // All three on separate lines.
    assert_eq!(summary.matches('\n').count(), 3);
}

#[test]
fn test_active_children_summary_mixed_states() {
    let session = ConversationSession::new("s_acs6".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "running task");
    session.register_child("c2", "agent-b", "done task");
    session.update_child_state("c2", ChildSessionState::Completed);
    let summary = session.active_children_summary().unwrap();
    assert!(summary.contains("agent-a"));
    assert!(summary.contains("running task"));
    assert!(!summary.contains("agent-b"));
    assert!(!summary.contains("done task"));
}

#[test]
fn test_active_children_summary_skips_none_detail() {
    let session = ConversationSession::new("s_acs7".into(), "gpt-4o".into(), tmp_path());
    // Manually insert a Running child with None detail (defensive).
    {
        let mut states = session.child_states.write().unwrap();
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
    let summary = session.active_children_summary().unwrap();
    assert!(summary.contains("agent-x"));
    assert!(summary.contains("real task"));
    // Header line + 1 item = 1 newline.
    assert_eq!(summary.matches('\n').count(), 1);
}

#[test]
fn test_active_children_summary_state_transition() {
    let session = ConversationSession::new("s_acs8".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "task 1");
    session.register_child("c2", "agent-b", "task 2");
    // Both active.
    let summary = session.active_children_summary().unwrap();
    assert!(summary.contains("agent-a"));
    assert!(summary.contains("agent-b"));
    // Complete one child.
    session.update_child_state("c1", ChildSessionState::Completed);
    let summary = session.active_children_summary().unwrap();
    assert!(!summary.contains("agent-a"));
    assert!(summary.contains("agent-b"));
    // Complete the last one.
    session.update_child_state("c2", ChildSessionState::Completed);
    assert!(session.active_children_summary().is_none());
}

#[test]
fn test_active_children_summary_yielded_session() {
    let session = ConversationSession::new("s_acs9".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "task 1");
    session.enter_waiting();
    // Summary still returns data even when yielded.
    let summary = session.active_children_summary();
    assert!(summary.is_some());
    assert!(summary.unwrap().contains("agent-a"));
}
