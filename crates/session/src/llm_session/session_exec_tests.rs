//! Unit tests for `exec_status()` state combinations (Step 1.6).
//!
//! Validates the four-dimensional execution state model against the
//! state table in `docs/design/session/session-execution.md`.

use super::*;
use closeclaw_common::{
    ChildSessionState, LlmState, SessionActivityDimensions, SessionExecStatus, ToolExecState,
};

// ── 1. foreground + child simultaneously active → Busy ──────────────────────

/// Per design doc: `foreground_tool_active=true → Busy` regardless
/// of `child_active`. Verify that Busy is returned when both are true.
#[test]
fn test_exec_status_fg_and_child_returns_busy() {
    let session = ConversationSession::new("s_fg_child".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.register_tool_call("tool-1", "bash", "cmd");
    session.update_tool_state("tool-1", ToolExecState::RunningForeground);

    assert_eq!(
        session.exec_status(),
        SessionExecStatus::Busy,
        "foreground tool active + child running → Busy"
    );
}

// ── 2. Only child running → Idle (not Waiting) ─────────────────────────────

/// Per design doc: child_active does NOT affect idle/Busy determination.
/// When only child is running (no yield, no LLM, no foreground tool),
/// exec_status returns Idle.
#[test]
fn test_exec_status_only_child_returns_idle() {
    let session = ConversationSession::new("s_child_only".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");

    assert_eq!(
        session.exec_status(),
        SessionExecStatus::Idle,
        "child_active alone must not cause Busy or Waiting"
    );
}

// ── 3. LLM + child → Busy (LLM overrides) ─────────────────────────────────

#[test]
fn test_exec_status_llm_and_child_returns_busy() {
    let session = ConversationSession::new("s_llm_child".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.set_llm_state(LlmState::Requesting);

    assert_eq!(
        session.exec_status(),
        SessionExecStatus::Busy,
        "LLM active + child running → Busy"
    );
}

// ── 4. All dimensions idle → Idle ──────────────────────────────────────────

#[test]
fn test_exec_status_all_idle() {
    let session = ConversationSession::new("s_all_idle".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);
}

// ── 5. Waiting only when yielding (is_yielding=true) ───────────────────────

/// Waiting is returned ONLY when `is_yielding=true` (agent called
/// `sessions_yield`). child_active alone does not cause Waiting.
#[test]
fn test_exec_status_waiting_requires_yielding() {
    let session = ConversationSession::new("s_yield_req".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");

    // Without yielding → Idle.
    assert_eq!(session.exec_status(), SessionExecStatus::Idle);

    // With yielding → Waiting.
    session.enter_waiting();
    assert_eq!(session.exec_status(), SessionExecStatus::Waiting);
}

// ── 6. LLM active overrides Waiting ────────────────────────────────────────

/// Even in Waiting state, if LLM becomes active → Busy.
#[test]
fn test_exec_status_llm_overrides_waiting() {
    let session = ConversationSession::new("s_llm_wait".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.enter_waiting();
    assert_eq!(session.exec_status(), SessionExecStatus::Waiting);

    // LLM starts → Busy.
    session.set_llm_state(LlmState::Requesting);
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// ── 7. Foreground tool overrides Waiting ────────────────────────────────────

#[test]
fn test_exec_status_fg_tool_overrides_waiting() {
    let session = ConversationSession::new("s_fg_wait".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.enter_waiting();
    assert_eq!(session.exec_status(), SessionExecStatus::Waiting);

    // Foreground tool starts → Busy.
    session.register_tool_call("tool-1", "bash", "cmd");
    assert_eq!(session.exec_status(), SessionExecStatus::Busy);
}

// ── 8. Background tool + child → Idle (neither blocks) ────────────────────

/// Per design doc: background_tool_active and child_active do NOT
/// affect idle/Busy. Both being true still results in Idle.
#[test]
fn test_exec_status_bg_tool_and_child_returns_idle() {
    let session = ConversationSession::new("s_bg_child".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.register_tool_call("bg-1", "bash", "ls");
    session.update_tool_state("bg-1", ToolExecState::RunningBackground);

    assert_eq!(
        session.exec_status(),
        SessionExecStatus::Idle,
        "background tool + child → Idle (per design doc)"
    );
}

// ── 9. Transition: child completes while foreground active ──────────────────

#[test]
fn test_exec_status_child_completes_fg_still_busy() {
    let session = ConversationSession::new("s_trans".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.register_tool_call("tool-1", "bash", "cmd");
    session.update_tool_state("tool-1", ToolExecState::RunningForeground);

    assert_eq!(session.exec_status(), SessionExecStatus::Busy);

    // Child completes.
    session.update_child_state("child-1", ChildSessionState::Completed);

    // Still Busy — foreground tool still active.
    assert_eq!(
        session.exec_status(),
        SessionExecStatus::Busy,
        "child completing must not affect Busy when fg tool is active"
    );
}

// ── 10. Transition: foreground completes, child still running → Idle ────────

#[test]
fn test_exec_status_fg_completes_child_still_idle() {
    let session = ConversationSession::new("s_trans2".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.register_tool_call("tool-1", "bash", "cmd");
    session.update_tool_state("tool-1", ToolExecState::RunningForeground);

    assert_eq!(session.exec_status(), SessionExecStatus::Busy);

    // Foreground tool completes.
    session.update_tool_state("tool-1", ToolExecState::Completed);

    // Child still running, but idle because no fg tool, no LLM.
    assert_eq!(
        session.exec_status(),
        SessionExecStatus::Idle,
        "fg completing while child running must return Idle"
    );
}

// ── 11. Yield + child + background tool → Waiting (yielding overrides) ─────

#[test]
fn test_exec_status_yield_with_bg_tool_returns_waiting() {
    let session = ConversationSession::new("s_y_bg".into(), "gpt-4o".into(), tmp_path());
    session.register_child("child-1", "agent-a", "task");
    session.register_tool_call("bg-1", "bash", "ls");
    session.update_tool_state("bg-1", ToolExecState::RunningBackground);
    session.enter_waiting();

    // Waiting because is_yielding=true, even with background tool + child.
    assert_eq!(session.exec_status(), SessionExecStatus::Waiting);
}

// ── 12. Pending tool (just registered) → Busy ──────────────────────────────

/// A newly registered tool in Pending state is treated as
/// foreground-active → Busy.
#[test]
fn test_exec_status_pending_tool_returns_busy() {
    let session = ConversationSession::new("s_pend_tool".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("tool-1", "bash", "echo");
    // Still in Pending state (no update_tool_state call).

    assert_eq!(
        session.exec_status(),
        SessionExecStatus::Busy,
        "Pending tool must be treated as foreground-active → Busy"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// activity_dimensions() tests
// ═══════════════════════════════════════════════════════════════════════════

/// (a) Empty session → all four dimensions false.
#[test]
fn test_activity_dimensions_empty_session_all_false() {
    let session = ConversationSession::new("s_empty".into(), "gpt-4o".into(), tmp_path());
    let dims = session.activity_dimensions();
    assert_eq!(
        dims,
        SessionActivityDimensions {
            llm_active: false,
            foreground_tool_active: false,
            background_tool_active: false,
            child_active: false,
        },
        "empty session must have all dimensions false"
    );
    assert!(
        !dims.any_active(),
        "any_active must be false when all are false"
    );
}

/// (b) LLM Requesting → llm_active=true, others false.
#[test]
fn test_activity_dimensions_llm_requesting() {
    let session = ConversationSession::new("s_llm_req".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Requesting);
    let dims = session.activity_dimensions();
    assert!(dims.llm_active, "llm_active must be true when Requesting");
    assert!(!dims.foreground_tool_active);
    assert!(!dims.background_tool_active);
    assert!(!dims.child_active);
    assert!(dims.any_active());
}

/// (b2) LLM Receiving → llm_active=true.
#[test]
fn test_activity_dimensions_llm_receiving() {
    let session = ConversationSession::new("s_llm_rec".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Receiving);
    let dims = session.activity_dimensions();
    assert!(dims.llm_active, "llm_active must be true when Receiving");
}

/// (c) Pending tool → foreground_tool_active=true.
#[test]
fn test_activity_dimensions_pending_tool_fg() {
    let session = ConversationSession::new("s_pend".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("t1", "bash", "echo");
    let dims = session.activity_dimensions();
    assert!(
        dims.foreground_tool_active,
        "Pending tool must set foreground_tool_active"
    );
    assert!(!dims.llm_active);
    assert!(!dims.background_tool_active);
    assert!(!dims.child_active);
}

/// (c2) RunningForeground tool → foreground_tool_active=true.
#[test]
fn test_activity_dimensions_running_fg_tool() {
    let session = ConversationSession::new("s_run_fg".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("t1", "bash", "cmd");
    session.update_tool_state("t1", ToolExecState::RunningForeground);
    let dims = session.activity_dimensions();
    assert!(
        dims.foreground_tool_active,
        "RunningForeground tool must set foreground_tool_active"
    );
}

/// (d) RunningBackground tool → background_tool_active=true.
#[test]
fn test_activity_dimensions_running_bg_tool() {
    let session = ConversationSession::new("s_run_bg".into(), "gpt-4o".into(), tmp_path());
    session.register_tool_call("t1", "bash", "ls");
    session.update_tool_state("t1", ToolExecState::RunningBackground);
    let dims = session.activity_dimensions();
    assert!(
        dims.background_tool_active,
        "RunningBackground tool must set background_tool_active"
    );
    assert!(!dims.foreground_tool_active, "bg must not set fg");
}

/// (e) Running child → child_active=true.
#[test]
fn test_activity_dimensions_running_child() {
    let session = ConversationSession::new("s_child".into(), "gpt-4o".into(), tmp_path());
    session.register_child("c1", "agent-a", "task");
    let dims = session.activity_dimensions();
    assert!(dims.child_active, "Running child must set child_active");
    assert!(!dims.llm_active);
    assert!(!dims.foreground_tool_active);
    assert!(!dims.background_tool_active);
}

/// Multiple dimensions active simultaneously.
#[test]
fn test_activity_dimensions_multiple_active() {
    let session = ConversationSession::new("s_multi".into(), "gpt-4o".into(), tmp_path());
    session.set_llm_state(LlmState::Requesting);
    session.register_tool_call("t1", "bash", "cmd");
    session.update_tool_state("t1", ToolExecState::RunningForeground);
    session.register_tool_call("t2", "bash", "ls");
    session.update_tool_state("t2", ToolExecState::RunningBackground);
    session.register_child("c1", "agent-a", "task");
    let dims = session.activity_dimensions();
    assert!(dims.llm_active);
    assert!(dims.foreground_tool_active);
    assert!(dims.background_tool_active);
    assert!(dims.child_active);
    assert!(dims.any_active());
}
