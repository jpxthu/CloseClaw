//! Unit tests for mode-switch checkpoint persistence (Step 1.2).
//!
//! Covers the behavior introduced in Step 1.1:
//! - `set_session_mode` writes new `session_mode` to checkpoint
//! - `apply_pending_session_mode_if_needed` (lazy apply) writes new
//!   `session_mode` to checkpoint
//! - Mode switch resets `mode_state` to default
//! - No checkpoint write when mode is unchanged
//! - No checkpoint write when `pending_session_mode` is None

use crate::llm_session::mode_transition::ModeChangeSource;
use crate::llm_session::ConversationSession;
use crate::persistence::{PersistenceService, ReasoningModeState, SessionCheckpoint, SessionMode};
use crate::storage::memory::MemoryStorage;
use std::sync::Arc;
use std::time::Duration;

/// Helper: create a ConversationSession with MemoryStorage wired up.
fn make_session(session_id: &str) -> (ConversationSession, Arc<MemoryStorage>, tempfile::TempDir) {
    let storage = Arc::new(MemoryStorage::new());
    let temp = tempfile::tempdir().expect("temp dir");
    let mut cs = ConversationSession::new(
        session_id.into(),
        "test-model".into(),
        temp.path().to_path_buf(),
    );
    cs.set_checkpoint_storage(Arc::clone(&storage) as Arc<dyn PersistenceService>);
    (cs, storage, temp)
}

/// Helper: load the checkpoint from storage.
async fn load_cp(storage: &MemoryStorage, id: &str) -> Option<SessionCheckpoint> {
    storage.load_checkpoint(id).await.unwrap()
}

/// Helper: build a checkpoint with non-default mode_state for testing reset.
fn checkpoint_with_mode_state(
    id: &str,
    mode: SessionMode,
    mode_state: ReasoningModeState,
) -> SessionCheckpoint {
    let mut cp = SessionCheckpoint::new(id.into());
    cp.session_mode = mode;
    cp.mode_state = mode_state;
    cp
}

/// Non-default ReasoningModeState for asserting reset.
fn dirty_mode_state() -> ReasoningModeState {
    ReasoningModeState {
        current_step: 3,
        total_steps: 5,
        step_messages: vec!["step1".into(), "step2".into()],
        is_complete: false,
    }
}

/// Wait for the spawned writeback task to complete by yielding to the
/// tokio runtime. Multiple yields ensure the spawned task is scheduled,
/// executes, and its save propagates to storage.
async fn await_writeback() {
    for _ in 0..5 {
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ── 1. set_session_mode persists new session_mode ────────────────────

#[tokio::test]
async fn test_set_session_mode_persists_new_mode() {
    let (mut cs, storage, _temp) = make_session("sess_set_mode");

    // Pre-seed checkpoint so writeback has something to update.
    let seed = SessionCheckpoint::new("sess_set_mode".into());
    storage.save_checkpoint(&seed).await.unwrap();

    cs.set_session_mode(SessionMode::Plan, ModeChangeSource::Manual);
    await_writeback().await;

    let cp = load_cp(&storage, "sess_set_mode")
        .await
        .expect("checkpoint should be persisted");
    assert_eq!(
        cp.session_mode,
        SessionMode::Plan,
        "checkpoint session_mode should be Plan after set_session_mode"
    );
}

// ── 2. lazy apply persists new session_mode ──────────────────────────

#[tokio::test]
async fn test_lazy_apply_persists_session_mode() {
    let (cs, storage, _temp) = make_session("sess_lazy_apply");

    // Pre-seed checkpoint so writeback has something to update.
    let seed = SessionCheckpoint::new("sess_lazy_apply".into());
    storage.save_checkpoint(&seed).await.unwrap();

    cs.set_pending_session_mode(SessionMode::Auto);
    // session_mode() triggers apply_pending_session_mode_if_needed
    let mode = cs.session_mode();
    assert_eq!(mode, SessionMode::Auto, "session_mode() should return Auto");

    await_writeback().await;

    let cp = load_cp(&storage, "sess_lazy_apply")
        .await
        .expect("checkpoint should be persisted after lazy apply");
    assert_eq!(
        cp.session_mode,
        SessionMode::Auto,
        "checkpoint session_mode should be Auto after lazy apply"
    );
}

// ── 3. plan exit resets mode_state ───────────────────────────────────

#[tokio::test]
async fn test_plan_exit_resets_mode_state() {
    let (mut cs, storage, _temp) = make_session("sess_plan_exit");

    // First set session to Plan (in-memory) so that the subsequent
    // Normal switch is a real mode transition.
    cs.set_session_mode(SessionMode::Plan, ModeChangeSource::Manual);
    await_writeback().await;

    // Now overwrite the checkpoint with dirty mode_state to simulate
    // stale reasoning state from a prior plan session.
    let cp = checkpoint_with_mode_state("sess_plan_exit", SessionMode::Plan, dirty_mode_state());
    storage.save_checkpoint(&cp).await.unwrap();

    // Switch from Plan → Normal (plan exit).
    cs.set_session_mode(SessionMode::Normal, ModeChangeSource::Manual);
    await_writeback().await;

    let cp = load_cp(&storage, "sess_plan_exit")
        .await
        .expect("checkpoint should exist");
    assert_eq!(
        cp.session_mode,
        SessionMode::Normal,
        "session_mode should be Normal after plan exit"
    );
    assert_eq!(
        cp.mode_state,
        ReasoningModeState::default(),
        "mode_state should be reset to default on plan exit"
    );
}

// ── 4. plan entry resets mode_state ──────────────────────────────────

#[tokio::test]
async fn test_plan_entry_resets_mode_state() {
    let (mut cs, storage, _temp) = make_session("sess_plan_entry");

    // Pre-seed checkpoint with Normal mode and dirty mode_state
    // (simulating stale state from a prior plan session).
    let cp = checkpoint_with_mode_state("sess_plan_entry", SessionMode::Normal, dirty_mode_state());
    storage.save_checkpoint(&cp).await.unwrap();

    // First call: Normal → Plan (sets has_been_in_plan = true).
    cs.set_session_mode(SessionMode::Plan, ModeChangeSource::Manual);
    await_writeback().await;

    // Switch out, then back in to trigger re-entry.
    cs.set_session_mode(SessionMode::Normal, ModeChangeSource::Manual);
    await_writeback().await;
    cs.set_session_mode(SessionMode::Plan, ModeChangeSource::Manual);
    await_writeback().await;

    let cp = load_cp(&storage, "sess_plan_entry")
        .await
        .expect("checkpoint should exist");
    assert_eq!(cp.session_mode, SessionMode::Plan);
    assert_eq!(
        cp.mode_state,
        ReasoningModeState::default(),
        "mode_state should be reset to default on plan re-entry"
    );
}

// ── 5. same-mode set_session_mode does not save ─────────────────────

#[tokio::test]
async fn test_same_mode_no_save() {
    let (mut cs, storage, _temp) = make_session("sess_same_mode_save");

    // Pre-seed checkpoint.
    let seed = SessionCheckpoint::new("sess_same_mode_save".into());
    storage.save_checkpoint(&seed).await.unwrap();
    let baseline = storage.save_count();

    // Set to Normal (same as default) — should NOT trigger writeback.
    cs.set_session_mode(SessionMode::Normal, ModeChangeSource::Manual);
    await_writeback().await;

    assert_eq!(
        storage.save_count(),
        baseline,
        "save should NOT be called when mode is unchanged"
    );
}

// ── 6. pending_session_mode None does not save ───────────────────────

#[tokio::test]
async fn test_pending_none_no_save() {
    let (cs, storage, _temp) = make_session("sess_pending_none");

    // Do not set any pending mode — session_mode() should not write.
    let mode = cs.session_mode();
    assert_eq!(mode, SessionMode::Normal, "default mode is Normal");
    await_writeback().await;

    let cp = load_cp(&storage, "sess_pending_none").await;
    assert!(
        cp.is_none(),
        "no checkpoint should be written when pending_session_mode is None"
    );
}

// ── 7. lazy apply from Normal → Plan resets mode_state ───────────────

#[tokio::test]
async fn test_lazy_apply_plan_entry_resets_mode_state() {
    let (cs, storage, _temp) = make_session("sess_lazy_plan");

    // Pre-seed with dirty mode_state.
    let cp = checkpoint_with_mode_state("sess_lazy_plan", SessionMode::Normal, dirty_mode_state());
    storage.save_checkpoint(&cp).await.unwrap();

    // Set pending to Plan, then trigger via session_mode().
    cs.set_pending_session_mode(SessionMode::Plan);
    let _ = cs.session_mode();
    await_writeback().await;

    let cp = load_cp(&storage, "sess_lazy_plan")
        .await
        .expect("checkpoint should exist");
    assert_eq!(cp.session_mode, SessionMode::Plan);
    assert_eq!(
        cp.mode_state,
        ReasoningModeState::default(),
        "mode_state should be reset on lazy apply plan entry"
    );
}

// ── 8. lazy apply same-mode does not save ───────────────────────────

#[tokio::test]
async fn test_lazy_apply_same_mode_no_save() {
    let (cs, storage, _temp) = make_session("sess_lazy_same_mode");

    // Pre-seed checkpoint with dirty mode_state.
    let cp = checkpoint_with_mode_state(
        "sess_lazy_same_mode",
        SessionMode::Normal,
        dirty_mode_state(),
    );
    storage.save_checkpoint(&cp).await.unwrap();
    let baseline = storage.save_count();

    // Set pending to Normal (same as current) — should NOT write back.
    cs.set_pending_session_mode(SessionMode::Normal);
    let _ = cs.session_mode();
    await_writeback().await;

    assert_eq!(
        storage.save_count(),
        baseline,
        "save should NOT be called when pending mode matches current mode"
    );
}
