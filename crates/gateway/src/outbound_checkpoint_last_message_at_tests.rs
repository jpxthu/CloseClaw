//! Tests for `Gateway::persist_outbound_checkpoint` last_message_at behavior.
//!
//! Verifies that `persist_outbound_checkpoint` updates `last_message_at`
//! and does NOT modify `last_user_activity_at`, for both existing and
//! new checkpoints.

use crate::{Gateway, GatewayConfig, Message, SessionManager};
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Mock persistence
// ---------------------------------------------------------------------------

/// In-memory mock that records every saved checkpoint.
#[derive(Default)]
struct MockPersist {
    saves: Mutex<Vec<SessionCheckpoint>>,
}

impl MockPersist {
    fn last_save(&self) -> Option<SessionCheckpoint> {
        self.saves.lock().unwrap().last().cloned()
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.saves.lock().unwrap().push(cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.saves.lock().unwrap().last().cloned())
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn invalidate_session(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn list_idle_sessions_for_agent(
        &self,
        _a: &str,
        _r: closeclaw_session::persistence::AgentRole,
        _m: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _a: &str,
        _r: closeclaw_session::persistence::AgentRole,
        _m: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-cp-lma".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

/// Create a Gateway with a CheckpointManager backed by MockPersist.
async fn setup_gw(persist: Arc<MockPersist>) -> (Gateway, Arc<SessionManager>, String) {
    let session_id = "sess-cp-lma-1".to_string();
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    // Map session → chat_id for outbound resolution.
    sm.sessions.write().await.insert(
        session_id.clone(),
        crate::Session {
            id: session_id.clone(),
            agent_id: "chat_cp_lma".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cm = Arc::new(CheckpointManager::new(
        Arc::clone(&persist) as Arc<dyn PersistenceService>
    ));
    let gw = Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);
    (gw, sm, session_id)
}

/// Register a ConversationSession for the given session_id.
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

/// Build a minimal outbound Message.
fn make_msg(_session_id: &str) -> Message {
    Message {
        id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
        from: "agent".to_string(),
        to: "chat_cp_lma".to_string(),
        content: "test outbound".to_string(),
        channel: "mock".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Existing checkpoint: persist_outbound_checkpoint updates last_message_at.
#[tokio::test]
async fn test_outbound_checkpoint_sets_last_message_at_on_existing() {
    let persist = Arc::new(MockPersist::default());
    let (gw, sm, session_id) = setup_gw(persist.clone()).await;
    register_conv_session(&sm, &session_id).await;

    // Pre-populate checkpoint with a known last_message_at.
    let mut old_cp = SessionCheckpoint::new(session_id.clone());
    old_cp.last_message_at = Some("2025-01-01T00:00:00Z".parse().unwrap());
    old_cp.last_user_activity_at = Some("2025-01-01T00:00:00Z".parse().unwrap());
    // Seed it via save_raw so the CheckpointManager cache has it.
    persist.saves.lock().unwrap().push(old_cp);

    let msg = make_msg(&session_id);
    gw.persist_outbound_checkpoint(&session_id, &msg, true)
        .await;

    // Wait for the spawned save task.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let saved = persist.last_save().expect("checkpoint should be saved");
    assert!(
        saved.last_message_at.is_some(),
        "last_message_at should be set"
    );
    let saved_ts = saved.last_message_at.unwrap();
    let old_ts: chrono::DateTime<chrono::Utc> = "2025-01-01T00:00:00Z".parse().unwrap();
    assert!(
        saved_ts > old_ts,
        "last_message_at should be updated to a later time"
    );
    // Verify last_user_activity_at is unchanged.
    assert_eq!(
        saved.last_user_activity_at,
        Some(old_ts),
        "last_user_activity_at should not be modified"
    );
}

/// New checkpoint (no prior checkpoint exists): last_message_at is set.
#[tokio::test]
async fn test_outbound_checkpoint_sets_last_message_at_on_new() {
    let persist = Arc::new(MockPersist::default());
    let (gw, sm, session_id) = setup_gw(persist.clone()).await;
    register_conv_session(&sm, &session_id).await;

    let msg = make_msg(&session_id);
    gw.persist_outbound_checkpoint(&session_id, &msg, true)
        .await;

    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let saved = persist.last_save().expect("checkpoint should be saved");
    assert!(
        saved.last_message_at.is_some(),
        "last_message_at should be set on new checkpoint"
    );
    // New checkpoint: last_user_activity_at should be None (not set by persist_outbound_checkpoint).
    assert!(
        saved.last_user_activity_at.is_none(),
        "last_user_activity_at should remain None on new checkpoint"
    );
}

/// mark_sent=false (pre-send): last_message_at is still set.
#[tokio::test]
async fn test_outbound_checkpoint_presend_sets_last_message_at() {
    let persist = Arc::new(MockPersist::default());
    let (gw, sm, session_id) = setup_gw(persist.clone()).await;
    register_conv_session(&sm, &session_id).await;

    let msg = make_msg(&session_id);
    gw.persist_outbound_checkpoint(&session_id, &msg, false)
        .await;

    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let saved = persist.last_save().expect("checkpoint should be saved");
    assert!(
        saved.last_message_at.is_some(),
        "last_message_at should be set even for pre-send persist"
    );
}

/// mark_sent=true (post-send): last_message_at is updated again.
#[tokio::test]
async fn test_outbound_checkpoint_postsend_updates_last_message_at() {
    let persist = Arc::new(MockPersist::default());
    let (gw, sm, session_id) = setup_gw(persist.clone()).await;
    register_conv_session(&sm, &session_id).await;

    let msg = make_msg(&session_id);

    // First persist (pre-send).
    gw.persist_outbound_checkpoint(&session_id, &msg, false)
        .await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let first_save = persist.last_save().unwrap();
    let first_lma = first_save.last_message_at.unwrap();

    // Second persist (post-send).
    gw.persist_outbound_checkpoint(&session_id, &msg, true)
        .await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let second_save = persist.last_save().unwrap();
    let second_lma = second_save.last_message_at.unwrap();
    assert!(
        second_lma >= first_lma,
        "post-send last_message_at should be >= pre-send"
    );
}
