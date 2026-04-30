//! Tests for archived session restore functionality.
//!
//! In the new architecture, session lifecycle is managed by SessionManager.
//! Gateway.route_message reads session_id from metadata (set by SessionRouter).
//! These tests verify that SessionManager.find_or_create correctly restores
//! archived sessions and sends notifications.

use crate::gateway::{GatewayConfig, Message, SessionManager};
use crate::im::IMAdapter;
use crate::session::persistence::{
    AgentRole, PersistenceService, SessionCheckpoint, SessionStatus,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ─────────────────────────────────────────────────────────────────────────────
// Mock persistence service
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct MockPersistenceService {
    checkpoints: RwLock<HashMap<String, SessionCheckpoint>>,
    archived: RwLock<HashMap<String, SessionCheckpoint>>,
    restored: RwLock<Vec<String>>,
}

impl MockPersistenceService {
    fn with_archived(session_id: &str, chat_id: &str) -> Arc<Self> {
        let cp = SessionCheckpoint::new(session_id.to_string())
            .with_status(SessionStatus::Archived)
            .with_chat_id(chat_id.to_string());
        let archived = RwLock::new(std::collections::HashMap::from([(
            session_id.to_string(),
            cp,
        )]));
        Arc::new(MockPersistenceService {
            archived,
            ..Default::default()
        })
    }

    fn with_active(session_id: &str, chat_id: &str) -> Arc<Self> {
        let cp = SessionCheckpoint::new(session_id.to_string())
            .with_status(SessionStatus::Active)
            .with_chat_id(chat_id.to_string());
        let checkpoints = RwLock::new(std::collections::HashMap::from([(
            session_id.to_string(),
            cp,
        )]));
        Arc::new(MockPersistenceService {
            checkpoints,
            ..Default::default()
        })
    }
}

#[async_trait]
impl PersistenceService for MockPersistenceService {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        self.checkpoints
            .write()
            .await
            .insert(checkpoint.session_id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, crate::session::persistence::PersistenceError> {
        if let Some(cp) = self.checkpoints.read().await.get(session_id).cloned() {
            return Ok(Some(cp));
        }
        Ok(self.archived.read().await.get(session_id).cloned())
    }

    async fn delete_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        self.checkpoints.write().await.remove(session_id);
        Ok(())
    }

    async fn list_active_sessions(
        &self,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(self.checkpoints.read().await.keys().cloned().collect())
    }

    async fn archive_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        self.archived
            .write()
            .await
            .insert(checkpoint.session_id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn restore_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, crate::session::persistence::PersistenceError> {
        let mut archived = self.archived.write().await;
        if let Some(cp) = archived.remove(session_id) {
            self.restored.write().await.push(session_id.to_string());
            self.checkpoints
                .write()
                .await
                .insert(cp.session_id.clone(), cp.clone());
            return Ok(Some(cp));
        }
        Ok(None)
    }

    async fn purge_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_archived_sessions(
        &self,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(self.archived.read().await.keys().cloned().collect())
    }

    async fn invalidate_session(
        &self,
        _session_id: &str,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _idle_minutes: i64,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(vec![])
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: AgentRole,
        _purge_after_minutes: i64,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(vec![])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Mock IM adapter
// ─────────────────────────────────────────────────────────────────────────────

struct MockAdapter {
    sent: RwLock<Vec<Message>>,
    fail_next: RwLock<bool>,
}

impl MockAdapter {
    fn new() -> Self {
        Self {
            sent: RwLock::new(Vec::new()),
            fail_next: RwLock::new(false),
        }
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IMAdapter for MockAdapter {
    fn name(&self) -> &str {
        "mock"
    }

    async fn handle_webhook(&self, _payload: &[u8]) -> Result<Message, crate::im::AdapterError> {
        unimplemented!()
    }

    async fn send_message(&self, message: &Message) -> Result<(), crate::im::AdapterError> {
        {
            let mut fail = self.fail_next.write().await;
            if *fail {
                *fail = false;
                return Err(crate::im::AdapterError::SendFailed(
                    "mock error".to_string(),
                ));
            }
        }
        self.sent.write().await.push(message.clone());
        Ok(())
    }

    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        true
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 10000,
        dm_scope: crate::gateway::DmScope::PerChannelPeer,
    }
}

fn make_message() -> Message {
    Message {
        id: "msg_1".to_string(),
        from: "user_1".to_string(),
        to: "agent_1".to_string(),
        content: "hello".to_string(),
        channel: "test_channel".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    }
}

/// Helper to create a Gateway with a SessionManager that uses the given storage.
async fn make_gateway_with_storage(
    storage: Arc<MockPersistenceService>,
) -> (crate::gateway::Gateway, Arc<SessionManager>) {
    let config = make_config();
    let session_manager = Arc::new(SessionManager::new(&config, Some(storage)));
    let gateway = crate::gateway::Gateway::new(config, Arc::clone(&session_manager));
    (gateway, session_manager)
}

/// Helper to create a message with session_id already set in metadata.
async fn make_message_with_session(session_manager: &SessionManager) -> Message {
    let msg = make_message();
    let session_id = session_manager
        .find_or_create("test_channel", &msg, None)
        .await
        .unwrap();
    let mut msg_with_session = msg;
    msg_with_session
        .metadata
        .insert("session_id".to_string(), session_id);
    msg_with_session
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_route_message_archived_session_restores() {
    let session_id = "test_channel:user_1:agent_1";
    let storage = MockPersistenceService::with_archived(session_id, "user_1");
    let adapter = Arc::new(MockAdapter::new());

    let (gateway, session_manager) = make_gateway_with_storage(storage.clone()).await;
    // Register adapter on gateway (for route_message) and session_manager (for notifications)
    gateway
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;
    session_manager
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;

    // SessionManager.find_or_create triggers archived session restoration
    let restored_before = storage.restored.read().await.len();
    let msg_with_session = make_message_with_session(&session_manager).await;
    gateway
        .route_message("test_channel", msg_with_session, None)
        .await
        .unwrap();
    let restored_after = storage.restored.read().await.len();

    assert_eq!(
        restored_after,
        restored_before + 1,
        "archived session should be restored"
    );
}

#[tokio::test]
async fn test_route_message_archived_session_sends_notification() {
    let session_id = "test_channel:user_1:agent_1";
    let storage = MockPersistenceService::with_archived(session_id, "user_1");
    let adapter = Arc::new(MockAdapter::new());

    let (gateway, session_manager) = make_gateway_with_storage(storage.clone()).await;
    gateway
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;
    session_manager
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;

    let msg_with_session = make_message_with_session(&session_manager).await;
    gateway
        .route_message("test_channel", msg_with_session, None)
        .await
        .unwrap();

    let sent = adapter.sent.read().await;
    assert!(
        sent.iter().any(|m| m.content == "正在恢复会话..."),
        "restore notification should be sent"
    );
}

#[tokio::test]
async fn test_route_message_active_session_no_restore() {
    let session_id = "test_channel:user_1:agent_1";
    let storage = MockPersistenceService::with_active(session_id, "user_1");
    let adapter = Arc::new(MockAdapter::new());

    let (gateway, session_manager) = make_gateway_with_storage(storage.clone()).await;
    gateway
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;
    session_manager
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;

    // find_or_create should NOT try to restore an active session
    let msg_with_session = make_message_with_session(&session_manager).await;
    gateway
        .route_message("test_channel", msg_with_session, None)
        .await
        .unwrap();

    let restored_count = storage.restored.read().await.len();
    assert_eq!(restored_count, 0, "active session should not be restored");
}

#[tokio::test]
async fn test_route_message_no_stored_session_creates_new() {
    let storage = Arc::new(MockPersistenceService::default());
    let adapter = Arc::new(MockAdapter::new());

    let (gateway, session_manager) = make_gateway_with_storage(storage.clone()).await;
    gateway
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;
    session_manager
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;

    let msg_with_session = make_message_with_session(&session_manager).await;
    gateway
        .route_message("test_channel", msg_with_session, None)
        .await
        .unwrap();

    let restored_count = storage.restored.read().await.len();
    assert_eq!(
        restored_count, 0,
        "no session should be restored when none exists"
    );
    let sessions = gateway.get_agent_sessions("agent_1").await;
    assert!(!sessions.is_empty(), "new session should be created");
}

#[tokio::test]
async fn test_restore_notification_failure_does_not_block() {
    let session_id = "test_channel:user_1:agent_1";
    let storage = MockPersistenceService::with_archived(session_id, "user_1");
    let adapter = Arc::new(MockAdapter::new());

    let (gateway, session_manager) = make_gateway_with_storage(storage.clone()).await;
    gateway
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;
    session_manager
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;

    // Make adapter fail on next send (notification failure)
    *adapter.fail_next.write().await = true;

    // Should not error out even if notification fails
    let msg_with_session = make_message_with_session(&session_manager).await;
    gateway
        .route_message("test_channel", msg_with_session, None)
        .await
        .unwrap();

    let restored_count = storage.restored.read().await.len();
    assert_eq!(
        restored_count, 1,
        "session should still be restored even if notification fails"
    );
}

#[tokio::test]
async fn test_no_storage_no_restore() {
    let config = make_config();
    let session_manager = Arc::new(SessionManager::new(&config, None));
    let gateway = crate::gateway::Gateway::new(config, Arc::clone(&session_manager));
    let adapter = Arc::new(MockAdapter::new());
    gateway
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;
    session_manager
        .register_adapter(
            "test_channel".to_string(),
            Arc::clone(&adapter) as Arc<dyn IMAdapter>,
        )
        .await;

    let msg_with_session = make_message_with_session(&session_manager).await;
    gateway
        .route_message("test_channel", msg_with_session, None)
        .await
        .unwrap();

    let sessions = gateway.get_agent_sessions("agent_1").await;
    assert!(
        !sessions.is_empty(),
        "new session should be created when no storage"
    );
}
