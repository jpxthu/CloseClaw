//! Tests for `ToolSession` trait impl persistence behavior.
//!
//! Verifies that `persist_pending_checkpoint` (called by
//! `register_tool_call`, `deregister_tool_call`,
//! `register_child_state`, `deregister_child_state`) completes
//! synchronously — the checkpoint is persisted **before** the
//! `.await` returns, so callers can assert checkpoint state
//! immediately without `tokio::time::sleep` or `thread::sleep`.

use crate::llm_session::tests::tmp_path;
use crate::llm_session::ConversationSession;
use crate::persistence::{
    PendingOperationType, PersistenceError, PersistenceService, SessionCheckpoint,
};
use std::sync::{Arc, Mutex};

use closeclaw_common::tool_session::ToolSession;

// ── Mock storage ───────────────────────────────────────────────────────

/// In-memory mock storage that records every saved checkpoint.
#[derive(Debug, Default)]
struct MockStorage {
    /// All checkpoints passed to `save_checkpoint`, in order.
    saves: Mutex<Vec<SessionCheckpoint>>,
    /// When `true`, `save_checkpoint` returns an error.
    fail_on_save: Mutex<bool>,
}

impl MockStorage {
    /// Return the last saved checkpoint (if any).
    fn last_checkpoint(&self) -> Option<SessionCheckpoint> {
        self.saves.lock().unwrap().last().cloned()
    }

    /// Return the number of times `save_checkpoint` was called.
    fn save_count(&self) -> usize {
        self.saves.lock().unwrap().len()
    }

    /// Make the next `save_checkpoint` call return an error.
    fn set_fail_on_save(&self, fail: bool) {
        *self.fail_on_save.lock().unwrap() = fail;
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockStorage {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        if *self.fail_on_save.lock().unwrap() {
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "mock save failure",
            )));
        }
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

// ── 1. Normal path: register_tool_call ──────────────────────────────────

/// After `register_tool_call`, the checkpoint should contain
/// a `ToolCall` entry in `pending_operations` — persisted
/// synchronously before the `.await` returns.
#[tokio::test]
async fn test_register_tool_call_persists_pending_operation() {
    let (session, storage) = session_with_storage("reg_persist");

    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_1".into(),
        "bash".into(),
        "echo hello".into(),
    )
    .await;

    // No sleep or yield needed — persist_completed before .await returned.
    let cp = storage.last_checkpoint().expect("checkpoint was saved");
    assert_eq!(cp.session_id, "reg_persist");

    let ops = &cp.pending_operations;
    assert_eq!(ops.len(), 1, "expected exactly one pending operation");
    assert_eq!(ops[0].op_id, "call_1");
    assert_eq!(ops[0].op_type, PendingOperationType::ToolCall);
}

// ── 2. Clear path: deregister_tool_call ─────────────────────────────────

/// After `deregister_tool_call`, the corresponding `ToolCall` entry
/// should be removed from `pending_operations`.
#[tokio::test]
async fn test_deregister_tool_call_removes_from_checkpoint() {
    let (session, storage) = session_with_storage("dereg_persist");

    // Register first.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_del".into(),
        "bash".into(),
        "rm -rf /tmp/test".into(),
    )
    .await;

    // Verify it's there immediately (no async wait).
    {
        let cp = storage.last_checkpoint().unwrap();
        assert_eq!(cp.pending_operations.len(), 1);
        assert_eq!(cp.pending_operations[0].op_id, "call_del");
    }

    // Deregister.
    <ConversationSession as ToolSession>::deregister_tool_call(&session, "call_del".into()).await;

    // Verify it's gone — persist completed before .await returned.
    let cp = storage.last_checkpoint().unwrap();
    let has_call_del = cp
        .pending_operations
        .iter()
        .any(|op| op.op_id == "call_del");
    assert!(
        !has_call_del,
        "call_del should have been removed from pending_operations \
         after deregister"
    );
}

// ── 3. Error path: storage write failure ────────────────────────────────

/// When `save_checkpoint` fails, `persist_pending_checkpoint` should
/// log a warning but NOT panic — graceful error handling.
#[tokio::test]
async fn test_persist_error_does_not_panic() {
    let (session, storage) = session_with_storage("err_persist");

    // Make the storage fail on save.
    storage.set_fail_on_save(true);

    // This must NOT panic.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_err".into(),
        "bash".into(),
        "echo fail".into(),
    )
    .await;

    // Verify no checkpoint was saved (failure path).
    assert_eq!(
        storage.save_count(),
        0,
        "no checkpoint should be saved on failure"
    );
}

/// After a failed persist, subsequent successful saves should work.
#[tokio::test]
async fn test_persist_error_recovery() {
    let (session, storage) = session_with_storage("err_recovery");

    // Fail first save.
    storage.set_fail_on_save(true);
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_err".into(),
        "bash".into(),
        "echo fail".into(),
    )
    .await;
    assert_eq!(storage.save_count(), 0);

    // Recover — disable failure.
    storage.set_fail_on_save(false);
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_ok".into(),
        "bash".into(),
        "echo ok".into(),
    )
    .await;

    assert_eq!(storage.save_count(), 1);
    let cp = storage.last_checkpoint().unwrap();
    let has_ok = cp.pending_operations.iter().any(|op| op.op_id == "call_ok");
    assert!(
        has_ok,
        "call_ok should be in pending_operations after recovery"
    );
}

// ── 4. Child session path ──────────────────────────────────────────────

/// `register_child_state` persists a `SubSessionSpawn` entry
/// synchronously — checkpoint available immediately.
#[tokio::test]
async fn test_register_child_state_persists_pending_operation() {
    let (session, storage) = session_with_storage("child_persist");

    <ConversationSession as ToolSession>::register_child_state(
        &session,
        "child_1".into(),
        "agent-a".into(),
        "do something".into(),
    )
    .await;

    let cp = storage.last_checkpoint().expect("checkpoint was saved");
    assert_eq!(cp.session_id, "child_persist");

    let ops = &cp.pending_operations;
    assert_eq!(ops.len(), 1, "expected exactly one pending operation");
    assert_eq!(ops[0].op_id, "child_1");
    assert_eq!(ops[0].op_type, PendingOperationType::SubSessionSpawn);
}

/// `deregister_child_state` removes the entry synchronously.
#[tokio::test]
async fn test_deregister_child_state_removes_from_checkpoint() {
    let (session, storage) = session_with_storage("child_dereg");

    // Register first.
    <ConversationSession as ToolSession>::register_child_state(
        &session,
        "child_del".into(),
        "agent-b".into(),
        "task".into(),
    )
    .await;

    {
        let cp = storage.last_checkpoint().unwrap();
        assert_eq!(cp.pending_operations.len(), 1);
        assert_eq!(cp.pending_operations[0].op_id, "child_del");
    }

    // Deregister.
    <ConversationSession as ToolSession>::deregister_child_state(&session, "child_del".into())
        .await;

    let cp = storage.last_checkpoint().unwrap();
    let has_child_del = cp
        .pending_operations
        .iter()
        .any(|op| op.op_id == "child_del");
    assert!(
        !has_child_del,
        "child_del should have been removed from pending_operations \
         after deregister_child_state"
    );
}

// ── 5. Cross-type consistency ──────────────────────────────────────────

/// Both `register_tool_call` and `register_child_state` trigger
/// persist — verify the symmetry.
#[tokio::test]
async fn test_register_tool_and_child_both_trigger_persist() {
    let (session, storage) = session_with_storage("symmetry");

    let saves_before = storage.save_count();

    // Register a tool call.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "tool_a".into(),
        "bash".into(),
        "ls".into(),
    )
    .await;

    let saves_after_tool = storage.save_count();
    assert!(
        saves_after_tool > saves_before,
        "register_tool_call should trigger persist"
    );

    // Register a child session.
    <ConversationSession as ToolSession>::register_child_state(
        &session,
        "child_1".into(),
        "agent-a".into(),
        "do something".into(),
    )
    .await;

    let saves_after_child = storage.save_count();
    assert!(
        saves_after_child > saves_after_tool,
        "register_child_state should trigger persist"
    );

    // The final checkpoint should contain both entries.
    let cp = storage.last_checkpoint().unwrap();
    let has_tool = cp
        .pending_operations
        .iter()
        .any(|op| op.op_id == "tool_a" && op.op_type == PendingOperationType::ToolCall);
    let has_child = cp
        .pending_operations
        .iter()
        .any(|op| op.op_id == "child_1" && op.op_type == PendingOperationType::SubSessionSpawn);
    assert!(has_tool, "tool_a should be in pending_operations");
    assert!(has_child, "child_1 should be in pending_operations");
}

/// Symmetry: deregister paths for tool and child both trigger persist
/// and remove the corresponding entry.
#[tokio::test]
async fn test_deregister_tool_and_child_both_trigger_persist() {
    let (session, storage) = session_with_storage("dereg_sym");

    // Register both.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "t1".into(),
        "bash".into(),
        "cmd".into(),
    )
    .await;
    <ConversationSession as ToolSession>::register_child_state(
        &session,
        "c1".into(),
        "agent-x".into(),
        "task".into(),
    )
    .await;

    {
        let cp = storage.last_checkpoint().unwrap();
        assert_eq!(cp.pending_operations.len(), 2);
    }

    let saves_before = storage.save_count();

    // Deregister the tool.
    <ConversationSession as ToolSession>::deregister_tool_call(&session, "t1".into()).await;

    let cp = storage.last_checkpoint().unwrap();
    assert!(
        cp.pending_operations.iter().all(|op| op.op_id != "t1"),
        "t1 should be removed"
    );
    // Child should still be there.
    assert!(
        cp.pending_operations.iter().any(|op| op.op_id == "c1"),
        "c1 should still be present"
    );

    let saves_after = storage.save_count();
    assert!(
        saves_after > saves_before,
        "deregister_tool_call should trigger persist"
    );

    // Deregister the child.
    <ConversationSession as ToolSession>::deregister_child_state(&session, "c1".into()).await;

    let cp = storage.last_checkpoint().unwrap();
    assert!(
        cp.pending_operations.is_empty(),
        "both entries should be removed after deregistering tool and child"
    );
}

// ── 6. Multiple operations ─────────────────────────────────────────────

/// Multiple register calls accumulate; each persist is synchronous
/// so the checkpoint reflects all registered operations.
#[tokio::test]
async fn test_multiple_register_calls_accumulate_in_checkpoint() {
    let (session, storage) = session_with_storage("multi_reg");

    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "t1".into(),
        "bash".into(),
        "echo 1".into(),
    )
    .await;
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "t2".into(),
        "bash".into(),
        "echo 2".into(),
    )
    .await;
    <ConversationSession as ToolSession>::register_child_state(
        &session,
        "c1".into(),
        "agent-y".into(),
        "spawn".into(),
    )
    .await;

    let cp = storage.last_checkpoint().unwrap();
    assert_eq!(
        cp.pending_operations.len(),
        3,
        "three operations should be registered"
    );

    let ids: Vec<&str> = cp
        .pending_operations
        .iter()
        .map(|op| op.op_id.as_str())
        .collect();
    assert!(ids.contains(&"t1"));
    assert!(ids.contains(&"t2"));
    assert!(ids.contains(&"c1"));
}

// ── 7. No storage configured ───────────────────────────────────────────

/// When `checkpoint_storage` is `None`, `persist_pending_checkpoint`
/// is a no-op — should not panic.
#[tokio::test]
async fn test_persist_no_storage_is_noop() {
    let session = ConversationSession::new("no_storage".into(), "gpt-4o".into(), tmp_path());
    // Intentionally NOT calling set_checkpoint_storage.

    // Must not panic.
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_nop".into(),
        "bash".into(),
        "echo noop".into(),
    )
    .await;

    <ConversationSession as ToolSession>::deregister_tool_call(&session, "call_nop".into()).await;

    <ConversationSession as ToolSession>::register_child_state(
        &session,
        "child_nop".into(),
        "agent-z".into(),
        "noop task".into(),
    )
    .await;

    <ConversationSession as ToolSession>::deregister_child_state(&session, "child_nop".into())
        .await;
}
