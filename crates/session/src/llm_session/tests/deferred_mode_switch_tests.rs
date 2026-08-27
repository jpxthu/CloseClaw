use super::*;
use crate::llm_session::mode_transition::ModeChangeSource;
use closeclaw_common::ModeTransition;

// ── Normal path: pending mode returned by session_mode() ───────────────────

#[test]
fn test_pending_mode_applied_on_next_session_mode_call() {
    let session = ConversationSession::new("sess_d1".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.session_mode(), SessionMode::default());

    session.set_pending_session_mode(SessionMode::Plan);
    // Next call should return the pending mode.
    assert_eq!(session.session_mode(), SessionMode::Plan);
}

#[test]
fn test_pending_mode_auto_applied_on_next_session_mode_call() {
    let session = ConversationSession::new("sess_d2".into(), "gpt-4o".into(), tmp_path());
    session.set_pending_session_mode(SessionMode::Auto);
    assert_eq!(session.session_mode(), SessionMode::Auto);
}

// ── Boundary: no pending mode leaves current mode unchanged ────────────────

#[test]
fn test_no_pending_mode_returns_current_mode() {
    let session = ConversationSession::new("sess_d3".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.session_mode(), SessionMode::default());
    assert_eq!(session.session_mode(), SessionMode::default());
}

#[test]
fn test_no_pending_mode_preserves_explicit_mode() {
    let session = ConversationSession::new("sess_d4".into(), "gpt-4o".into(), tmp_path())
        .with_session_mode(SessionMode::Plan);
    // No pending mode set — should return the explicitly set mode.
    assert_eq!(session.session_mode(), SessionMode::Plan);
    assert_eq!(session.session_mode(), SessionMode::Plan);
}

// ── State transition: pending cleared after application ────────────────────

#[test]
fn test_pending_cleared_after_application() {
    let session = ConversationSession::new("sess_d5".into(), "gpt-4o".into(), tmp_path());
    session.set_pending_session_mode(SessionMode::Auto);
    // First call applies and clears pending.
    assert_eq!(session.session_mode(), SessionMode::Auto);
    // Second call should still return Auto (no pending re-applied).
    assert_eq!(session.session_mode(), SessionMode::Auto);
}

#[test]
fn test_pending_cleared_even_if_same_as_current() {
    let session = ConversationSession::new("sess_d6".into(), "gpt-4o".into(), tmp_path())
        .with_session_mode(SessionMode::Plan);
    session.set_pending_session_mode(SessionMode::Plan);
    // Pending should be consumed even though mode is identical.
    assert_eq!(session.session_mode(), SessionMode::Plan);
    // Subsequent call still returns Plan without issue.
    assert_eq!(session.session_mode(), SessionMode::Plan);
}

// ── Mode transition detection: Normal→Plan with has_been_in_plan ───────────

#[test]
fn test_pending_normal_to_plan_reentry_when_has_been_in_plan() {
    let mut session = ConversationSession::new("sess_d7".into(), "gpt-4o".into(), tmp_path());
    // Simulate prior Plan visit via immediate set (bypasses pending).
    session.set_session_mode(SessionMode::Plan, ModeChangeSource::Manual);
    session.set_session_mode(SessionMode::Normal, ModeChangeSource::Manual);
    // Consume any leftover transition.
    let _ = session.pending_mode_transition.lock().expect("lock").take();

    // Now set a pending mode: Normal → Plan with has_been_in_plan=true.
    session.set_pending_session_mode(SessionMode::Plan);
    let _ = session.session_mode(); // apply pending

    let transition = session.pending_mode_transition.lock().expect("lock").take();
    assert_eq!(transition, Some(ModeTransition::PlanModeReentry));
}

#[test]
fn test_pending_normal_to_plan_no_transition_first_entry() {
    let session = ConversationSession::new("sess_d8".into(), "gpt-4o".into(), tmp_path());
    // First ever entry: has_been_in_plan is false.
    session.set_pending_session_mode(SessionMode::Plan);
    let _ = session.session_mode(); // apply pending

    let transition = session.pending_mode_transition.lock().expect("lock").take();
    assert_eq!(transition, None);
}

#[test]
fn test_has_been_in_plan_set_when_pending_applies_plan() {
    let session = ConversationSession::new("sess_d9".into(), "gpt-4o".into(), tmp_path());
    assert!(!session.has_been_in_plan.load(Ordering::Relaxed));

    session.set_pending_session_mode(SessionMode::Plan);
    let _ = session.session_mode();

    assert!(session.has_been_in_plan.load(Ordering::Relaxed));
}

// ── Concurrency safety: multiple session_mode() calls apply once ───────────

#[test]
fn test_multiple_session_mode_calls_only_apply_pending_once() {
    let session = ConversationSession::new("sess_d10".into(), "gpt-4o".into(), tmp_path());
    session.set_pending_session_mode(SessionMode::Auto);

    // Call session_mode() multiple times.
    let m1 = session.session_mode();
    let m2 = session.session_mode();
    let m3 = session.session_mode();

    assert_eq!(m1, SessionMode::Auto);
    assert_eq!(m2, SessionMode::Auto);
    assert_eq!(m3, SessionMode::Auto);

    // Pending should have been consumed on the first call only.
    let pending = session.pending_session_mode.lock().expect("lock");
    assert!(pending.is_none());
}

// ── Automatic mode changes bypass pending ──────────────────────────────────

#[test]
fn test_automatic_mode_change_still_uses_immediate_set() {
    let mut session = ConversationSession::new("sess_d11".into(), "gpt-4o".into(), tmp_path());
    // Automatic source (e.g., auto execution finished) applies immediately.
    session.set_session_mode(SessionMode::Normal, ModeChangeSource::Automatic);
    assert_eq!(session.session_mode(), SessionMode::Normal);

    // No pending should have been set.
    let pending = session.pending_session_mode.lock().expect("lock");
    assert!(pending.is_none());
}

// ── Pending override: setting new pending before first read replaces old ───

#[test]
fn test_setting_new_pending_overwrites_previous() {
    let session = ConversationSession::new("sess_d12".into(), "gpt-4o".into(), tmp_path());
    session.set_pending_session_mode(SessionMode::Auto);
    session.set_pending_session_mode(SessionMode::Plan);
    // Only the last pending should apply.
    assert_eq!(session.session_mode(), SessionMode::Plan);
}

// ── Pending from Normal to Auto (no transition expected) ───────────────────

#[test]
fn test_pending_normal_to_auto_no_transition() {
    let session = ConversationSession::new("sess_d13".into(), "gpt-4o".into(), tmp_path());
    session.set_pending_session_mode(SessionMode::Auto);
    let _ = session.session_mode(); // apply pending

    let transition = session.pending_mode_transition.lock().expect("lock").take();
    assert_eq!(transition, None);
}
