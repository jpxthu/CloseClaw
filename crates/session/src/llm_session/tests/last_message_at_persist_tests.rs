//! Tests for `last_message_at` update behavior in checkpoint persistence.
//!
//! Verifies that:
//! - `persist_pending_checkpoint` updates `last_message_at` on existing checkpoints
//! - `persist_pending_checkpoint` sets `last_message_at` from None
//! - `persist_pending_checkpoint` does NOT modify `last_user_activity_at`
//! - `SessionCheckpoint` behavior: `last_message_at` set correctly in save paths

use crate::llm_session::tests::tmp_path;
use crate::llm_session::ConversationSession;
use crate::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use chrono::{Duration, Utc};
use std::sync::{Arc, Mutex};

use closeclaw_common::tool_session::ToolSession;

// ── Mock storage ────────────────────────────────────────────────────────────

/// In-memory mock storage that records every saved checkpoint.
#[derive(Debug, Default)]
struct MockStorage {
    saves: Mutex<Vec<SessionCheckpoint>>,
}

impl MockStorage {
    fn last_checkpoint(&self) -> Option<SessionCheckpoint> {
        self.saves.lock().unwrap().last().cloned()
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockStorage {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        self.saves.lock().unwrap().push(checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.last_checkpoint())
    }

    async fn delete_checkpoint(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn purge_checkpoint(&self, _id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
}

/// Create a `ConversationSession` wired up with `MockStorage`.
fn session_with_storage(session_id: &str) -> (ConversationSession, Arc<MockStorage>) {
    let storage = Arc::new(MockStorage::default());
    let mut session = ConversationSession::new(session_id.into(), "gpt-4o".into(), tmp_path());
    let storage_trait: Arc<dyn PersistenceService> =
        Arc::clone(&storage) as Arc<dyn PersistenceService>;
    session.set_checkpoint_storage(storage_trait);
    (session, storage)
}

/// Wait for the `tokio::spawn` inside `persist_pending_checkpoint` to finish.
async fn wait_for_persist() {
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

// ── Normal path: last_message_at updated on existing checkpoint ──────────────

/// When a checkpoint already exists with an older `last_message_at`,
/// `persist_pending_checkpoint` should update it to a newer value.
#[tokio::test]
async fn test_persist_pending_updates_last_message_at_on_existing_checkpoint() {
    let (session, storage) = session_with_storage("lma_update");

    // Pre-populate storage with a checkpoint that has an old last_message_at.
    let mut old_cp = SessionCheckpoint::new("lma_update".into());
    let old_time = Utc::now() - Duration::hours(1);
    old_cp.last_message_at = Some(old_time);
    old_cp.last_user_activity_at = Some(old_time);
    // Save directly to bypass cache.
    storage.saves.lock().unwrap().push(old_cp);

    // Trigger persist via ToolSession trait.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_1".into(),
        "bash".into(),
        "echo test".into(),
    )
    .await;
    wait_for_persist().await;

    let cp = storage.last_checkpoint().expect("checkpoint was saved");

    // last_message_at should be updated to a time > old_time.
    let new_time = cp.last_message_at.expect("last_message_at should be set");
    assert!(
        new_time > old_time,
        "last_message_at should be updated: got {:?}, expected > {:?}",
        new_time,
        old_time
    );

    // last_user_activity_at should remain unchanged.
    assert_eq!(
        cp.last_user_activity_at,
        Some(old_time),
        "last_user_activity_at should not be modified by persist_pending_checkpoint"
    );
}

// ── Boundary: last_message_at was None → correctly set ──────────────────────

/// When `last_message_at` is `None` on the existing checkpoint,
/// `persist_pending_checkpoint` should set it to `Some(now)`.
#[tokio::test]
async fn test_persist_pending_sets_last_message_at_from_none() {
    let (session, storage) = session_with_storage("lma_none");

    // Pre-populate storage with a checkpoint where last_message_at is None.
    let mut old_cp = SessionCheckpoint::new("lma_none".into());
    assert!(
        old_cp.last_message_at.is_none(),
        "new checkpoint should start with last_message_at = None"
    );
    old_cp.last_user_activity_at = None;
    storage.saves.lock().unwrap().push(old_cp);

    // Trigger persist.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_none".into(),
        "bash".into(),
        "pwd".into(),
    )
    .await;
    wait_for_persist().await;

    let cp = storage.last_checkpoint().expect("checkpoint was saved");

    // last_message_at should now be set.
    assert!(
        cp.last_message_at.is_some(),
        "last_message_at should be set from None to Some(now)"
    );

    // The value should be approximately "now" (within a few seconds).
    let now = Utc::now();
    let lma = cp.last_message_at.unwrap();
    let diff = (now - lma).num_seconds().abs();
    assert!(
        diff < 5,
        "last_message_at should be close to now, diff = {}s",
        diff
    );

    // last_user_activity_at should still be None (not touched).
    assert!(
        cp.last_user_activity_at.is_none(),
        "last_user_activity_at should remain None"
    );
}

// ── Behavior: last_user_activity_at is NOT modified ─────────────────────────

/// Verify that `persist_pending_checkpoint` never writes to `last_user_activity_at`,
/// regardless of its initial value.
#[tokio::test]
async fn test_persist_pending_does_not_modify_last_user_activity_at() {
    let (session, storage) = session_with_storage("lua_unchanged");

    // Set up a checkpoint with a specific last_user_activity_at.
    let mut old_cp = SessionCheckpoint::new("lua_unchanged".into());
    let user_activity_time = Utc::now() - Duration::days(3);
    old_cp.last_user_activity_at = Some(user_activity_time);
    old_cp.last_message_at = None;
    storage.saves.lock().unwrap().push(old_cp);

    // Trigger persist.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_lua".into(),
        "bash".into(),
        "ls".into(),
    )
    .await;
    wait_for_persist().await;

    let cp = storage.last_checkpoint().expect("checkpoint was saved");

    // last_user_activity_at must be unchanged.
    assert_eq!(
        cp.last_user_activity_at,
        Some(user_activity_time),
        "persist_pending_checkpoint must not modify last_user_activity_at"
    );

    // last_message_at should be set (proving the persist ran).
    assert!(
        cp.last_message_at.is_some(),
        "last_message_at should be updated to confirm persist ran"
    );
}

// ── Normal path: second persist updates last_message_at again ───────────────

/// Two consecutive `persist_pending_checkpoint` calls should produce
/// strictly increasing `last_message_at` values.
#[tokio::test]
async fn test_persist_pending_increments_last_message_at() {
    let (session, storage) = session_with_storage("lma_incr");

    // Pre-populate with a checkpoint.
    let mut old_cp = SessionCheckpoint::new("lma_incr".into());
    old_cp.last_message_at = Some(Utc::now() - Duration::hours(1));
    storage.saves.lock().unwrap().push(old_cp);

    // First persist.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_a".into(),
        "bash".into(),
        "a".into(),
    )
    .await;
    wait_for_persist().await;

    let cp1 = storage.last_checkpoint().unwrap();
    let t1 = cp1.last_message_at.unwrap();

    // Second persist.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_b".into(),
        "bash".into(),
        "b".into(),
    )
    .await;
    wait_for_persist().await;

    let cp2 = storage.last_checkpoint().unwrap();
    let t2 = cp2.last_message_at.unwrap();

    assert!(
        t2 > t1,
        "second persist should produce a strictly later last_message_at: {:?} <= {:?}",
        t2,
        t1
    );
}

// ── Checkpoint.new() starts with last_message_at = None ─────────────────────

/// Verify the default state of a freshly created `SessionCheckpoint`.
#[test]
fn test_new_checkpoint_has_last_message_at_none() {
    let cp = SessionCheckpoint::new("default_lma".into());
    assert!(
        cp.last_message_at.is_none(),
        "new SessionCheckpoint should have last_message_at = None"
    );
    assert!(
        cp.last_user_activity_at.is_none(),
        "new SessionCheckpoint should have last_user_activity_at = None"
    );
}
