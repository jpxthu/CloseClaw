//! Tests for `exec_status()` state combinations — Step 1.6.
//!
//! Focuses on the key state combinations modified by Steps 1.1–1.5:
//! - foreground + child active → Busy
//! - only child (no yielding) → Idle
//! - child + yielding → Waiting
//! - background tool + child → Idle (per design doc)

use super::super::*;
use closeclaw_common::{LlmState, SessionExecStatus, ToolExecState};

// ── foreground + child → Busy ──────────────────────────────────────────────

/// When a foreground tool is active AND a child session is running,
/// exec_status should return Busy (foreground overrides everything).
#[test]
fn test_exec_status_fg_tool_and_child_running_busy() {
    let session = ConversationSession::new("se_fg_child".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1", "bash", "echo");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    session.register_child("child_1", "agent-a", "task");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

/// Pending tool (not yet running) + child → Busy (pending counts as
/// foreground-active per design doc).
#[test]
fn test_exec_status_pending_tool_and_child_busy() {
    let session = ConversationSession::new("se_pend_child".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1", "bash", "echo");
    session.register_child("child_1", "agent-a", "task");
    // Pending tool is treated as foreground-active → Busy.
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// ── only child (no yielding) → Idle ────────────────────────────────────────

/// When only a child session is running (no LLM, no foreground tool,
/// not yielding), exec_status should return Idle per the design doc.
#[test]
fn test_exec_status_child_only_no_yield_idle() {
    let session = ConversationSession::new("se_child_only".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "task");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

/// Multiple children running, no other activity → still Idle.
#[test]
fn test_exec_status_multiple_children_no_yield_idle() {
    let session = ConversationSession::new("se_multi_child".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "task 1");
    session.register_child("c2", "agent-b", "task 2");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

// ── child + yielding → Waiting ─────────────────────────────────────────────

/// Child running + yielding → Waiting (only waiting condition for
/// child sessions).
#[test]
fn test_exec_status_child_and_yielding_waiting() {
    let session = ConversationSession::new("se_yield_child".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child_1", "agent-a", "task");
    session.enter_waiting();
    assert_eq!(session.exec_status(), SessionExecStatus::Waiting);
}

// ── background tool + child → Idle (per design doc) ──────────────────────

/// Background tool + child → Idle (child doesn't
/// affect status, background_tool_active does not affect idle
/// per design doc state table row 2).
#[test]
fn test_exec_status_bg_tool_and_child() {
    let session = ConversationSession::new("se_bg_child".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("bg_1", "bash", "ls");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    session.register_child("child_1", "agent-a", "task");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

// ── LLM + child → Busy ────────────────────────────────────────────────────

/// LLM requesting + child running → Busy (LLM dimension takes
/// precedence).
#[test]
fn test_exec_status_llm_requesting_and_child_busy() {
    let session = ConversationSession::new("se_llm_child".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Requesting);
    session.register_child("child_1", "agent-a", "task");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

/// LLM receiving + child + background tool → Busy (LLM dimension
/// takes precedence over all).
#[test]
fn test_exec_status_llm_receiving_child_bg_busy() {
    let session = ConversationSession::new("se_llm_bg_child".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Receiving);
    session.register_tool_call("bg_1", "bash", "ls");
    session.update_tool_state("bg_1", ToolExecState::RunningBackground);
    session.register_child("child_1", "agent-a", "task");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// ── foreground + child + yielding → Busy (foreground overrides) ────────────

/// When both foreground tool and child are active with yielding,
/// foreground tool makes it Busy, not Waiting.
#[test]
fn test_exec_status_fg_child_yielding_busy() {
    let session = ConversationSession::new("se_fg_child_y".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("call_1", "bash", "echo");
    session.update_tool_state("call_1", ToolExecState::RunningForeground);
    session.register_child("child_1", "agent-a", "task");
    session.enter_waiting();
    // Foreground tool overrides yielding → Busy.
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// ── all cleared → Idle ─────────────────────────────────────────────────────

/// After deregistering all tools and children, exec_status returns Idle.
#[test]
fn test_exec_status_all_cleared_idle() {
    let session = ConversationSession::new("se_clear".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("t1", "bash", "cmd");
    session.update_tool_state("t1", ToolExecState::RunningForeground);
    session.register_child("c1", "agent-a", "task");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);

    session.deregister_tool_call("t1");
    session.deregister_child("c1");
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}
