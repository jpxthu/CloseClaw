//! Tests for `SessionManager::save_checkpoint_after_compact`.

use crate::{GatewayConfig, SessionManager};
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::persistence::{PendingMessage, PersistenceService, SessionCheckpoint};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Mock persistence service ─────────────────────────────────────────────────

#[derive(Default)]
struct MockPersistence {
    checkpoints: RwLock<std::collections::HashMap<String, SessionCheckpoint>>,
}

#[async_trait::async_trait]
impl PersistenceService for MockPersistence {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        self.checkpoints
            .write()
            .await
            .insert(checkpoint.session_id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, closeclaw_session::persistence::PersistenceError> {
        Ok(self.checkpoints.read().await.get(session_id).cloned())
    }

    async fn delete_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(
        &self,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, closeclaw_session::persistence::PersistenceError> {
        Ok(None)
    }

    async fn purge_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_archived_sessions(
        &self,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn invalidate_session(
        &self,
        _session_id: &str,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: closeclaw_session::persistence::AgentRole,
        _idle_minutes: i64,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: closeclaw_session::persistence::AgentRole,
        _purge_after_minutes: i64,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 10000,
        ..Default::default()
    }
}

async fn make_sm_with_storage(persistence: Arc<MockPersistence>) -> SessionManager {
    SessionManager::new(
        &make_config(),
        Some(persistence as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    )
}

async fn register_conv_session(sm: &SessionManager, session_id: &str) {
    let cs = ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    let arc = Arc::new(RwLock::new(cs));
    sm.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), arc);
}

async fn push_pending(sm: &SessionManager, session_id: &str, msg: PendingMessage) {
    let cs = sm
        .conversation_sessions
        .read()
        .await
        .get(session_id)
        .expect("session should exist")
        .clone();
    cs.write().await.push_pending(msg);
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Normal path: after compaction, outbound_pending is synced to checkpoint.
#[tokio::test]
async fn test_save_checkpoint_after_compact_syncs_outbound_pending() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;
    let session_id = "test-session-1";

    // Register ConversationSession with pending messages (simulating post-compaction)
    register_conv_session(&sm, session_id).await;
    push_pending(
        &sm,
        session_id,
        PendingMessage::new("boundary-1".into(), "summary after compact".into()),
    )
    .await;
    push_pending(
        &sm,
        session_id,
        PendingMessage::new("boundary-2".into(), "second boundary".into()),
    )
    .await;

    // Pre-populate checkpoint in storage (as if it existed before compaction)
    let mut old_cp = SessionCheckpoint::new(session_id.to_string());
    old_cp.touch();
    persistence
        .checkpoints
        .write()
        .await
        .insert(session_id.to_string(), old_cp);

    // Act
    sm.save_checkpoint_after_compact(session_id).await;

    // Assert: checkpoint's outbound_pending matches ConversationSession
    let saved = persistence
        .checkpoints
        .read()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should be saved");
    assert_eq!(
        saved.outbound_pending.len(),
        2,
        "checkpoint should have 2 pending messages after compaction"
    );
    assert_eq!(saved.outbound_pending[0].message_id, "boundary-1");
    assert_eq!(saved.outbound_pending[0].content, "summary after compact");
    assert_eq!(saved.outbound_pending[1].message_id, "boundary-2");
    assert_eq!(saved.outbound_pending[1].content, "second boundary");
}

/// Boundary: ConversationSession does not exist — method returns silently.
#[tokio::test]
async fn test_save_checkpoint_after_compact_no_session_returns_silently() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;

    // Pre-populate checkpoint in storage
    let cp = SessionCheckpoint::new("nonexistent-session".to_string());
    persistence
        .checkpoints
        .write()
        .await
        .insert("nonexistent-session".to_string(), cp);

    // Act: session_id has no ConversationSession — should not panic
    sm.save_checkpoint_after_compact("nonexistent-session")
        .await;

    // Assert: checkpoint still saved (outbound_pending unchanged from original)
    let saved = persistence
        .checkpoints
        .read()
        .await
        .get("nonexistent-session")
        .cloned()
        .expect("checkpoint should still be saved");
    assert!(
        saved.outbound_pending.is_empty(),
        "outbound_pending should remain empty when no ConversationSession exists"
    );
}

/// Boundary: storage not initialized — method returns silently.
#[tokio::test]
async fn test_save_checkpoint_after_compact_no_storage_returns_silently() {
    let sm = SessionManager::new(
        &make_config(),
        None, // no storage
        None,
        ReasoningLevel::default(),
    );

    // Act: should not panic when storage is None
    sm.save_checkpoint_after_compact("any-session").await;
}
