//! Tests for `ToolSession` trait impl persistence behavior.
//!
//! Verifies that `register_tool_call` and `deregister_tool_call`
//! trigger `persist_pending_checkpoint`, keeping pending operations
//! consistent in the checkpoint — matching `register_child_state` /
//! `deregister_child_state`.

use crate::llm_session::tests::tmp_path;
use crate::llm_session::ConversationSession;
use crate::persistence::{
    PendingOperationType, PersistenceError, PersistenceService, SessionCheckpoint,
};
use std::sync::{Arc, Mutex};

use closeclaw_common::tool_session::ToolSession;

/// In-memory mock storage that records every saved checkpoint.
#[derive(Debug, Default)]
struct MockStorage {
    /// All checkpoints passed to `save_checkpoint`, in order.
    saves: Mutex<Vec<SessionCheckpoint>>,
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

/// Wait for the `tokio::spawn` inside `persist_pending_checkpoint` to
/// finish. Yields the runtime a few times so the spawned task can
/// complete its async work.
async fn wait_for_persist() {
    // The persist task is a tokio::spawn; yield the current task so the
    // runtime can schedule and complete it.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    // Small sleep as a safety net for any I/O-bound work in the future.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

// ── 1. Register persistence ───────────────────────────────────────────────

/// After `register_tool_call`, the checkpoint should contain
/// a `ToolCall` entry in `pending_operations`.
#[tokio::test]
async fn test_register_tool_call_persists_pending_operation() {
    let (session, storage) = session_with_storage("reg_persist");

    // Use the ToolSession trait method (which calls persist internally).
    <ConversationSession as ToolSession>::register_tool_call(
        &session,
        "call_1".into(),
        "bash".into(),
        "echo hello".into(),
    )
    .await;

    wait_for_persist().await;

    let cp = storage.last_checkpoint().expect("checkpoint was saved");
    assert_eq!(cp.session_id, "reg_persist");

    let ops = &cp.pending_operations;
    assert_eq!(ops.len(), 1, "expected exactly one pending operation");
    assert_eq!(ops[0].op_id, "call_1");
    assert_eq!(ops[0].op_type, PendingOperationType::ToolCall);
}

// ── 2. Deregister persistence ────────────────────────────────────────────

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
    wait_for_persist().await;

    // Verify it's there.
    {
        let cp = storage.last_checkpoint().unwrap();
        assert_eq!(cp.pending_operations.len(), 1);
        assert_eq!(cp.pending_operations[0].op_id, "call_del");
    }

    // Deregister.
    <ConversationSession as ToolSession>::deregister_tool_call(&session, "call_del".into()).await;
    wait_for_persist().await;

    // Verify it's gone.
    let cp = storage.last_checkpoint().unwrap();
    let has_call_del = cp
        .pending_operations
        .iter()
        .any(|op| op.op_id == "call_del");
    assert!(
        !has_call_del,
        "call_del should have been removed from pending_operations after deregister"
    );
}

// ── 3. Consistency with child state ───────────────────────────────────────

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
    wait_for_persist().await;

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
    wait_for_persist().await;

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
    wait_for_persist().await;

    {
        let cp = storage.last_checkpoint().unwrap();
        assert_eq!(cp.pending_operations.len(), 2);
    }

    let saves_before = storage.save_count();

    // Deregister the tool.
    <ConversationSession as ToolSession>::deregister_tool_call(&session, "t1".into()).await;
    wait_for_persist().await;

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
    wait_for_persist().await;

    let cp = storage.last_checkpoint().unwrap();
    assert!(
        cp.pending_operations.is_empty(),
        "both entries should be removed after deregistering tool and child"
    );
}
