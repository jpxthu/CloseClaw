//! Tests for CheckpointManager integration in Gateway.
//!
//! Verifies that `send_outbound` correctly triggers checkpoint persistence
//! when a CheckpointManager is configured.

use crate::gateway::{DmScope, GatewayConfig, Message, SessionManager};
use crate::im::{AdapterError, IMAdapter};
use crate::session::checkpoint_manager::CheckpointManager;
use crate::session::persistence::{PendingMessage, PersistenceService, SessionCheckpoint};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Mock adapter ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct MockAdapter {
    /// Whether `send_message` should fail.
    fail_send: bool,
    /// All messages sent through `send_message`.
    sent_messages: RwLock<Vec<Message>>,
}

impl MockAdapter {
    fn new() -> Self {
        Self::default()
    }

    async fn sent(&self) -> Vec<Message> {
        self.sent_messages.read().await.clone()
    }
}

#[async_trait]
impl IMAdapter for MockAdapter {
    fn name(&self) -> &str {
        "mock"
    }

    async fn handle_webhook(&self, _payload: &[u8]) -> Result<Message, AdapterError> {
        unimplemented!()
    }

    async fn send_message(&self, message: &Message) -> Result<(), AdapterError> {
        if self.fail_send {
            return Err(AdapterError::SendFailed("mock error".into()));
        }
        self.sent_messages.write().await.push(message.clone());
        Ok(())
    }

    async fn send_card_json(&self, _chat_id: &str, _card_json: &str) -> Result<(), AdapterError> {
        if self.fail_send {
            return Err(AdapterError::SendFailed("mock error".into()));
        }
        Ok(())
    }

    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        true
    }
}

// ── Mock persistence service ─────────────────────────────────────────────────

#[derive(Default)]
struct MockPersistence {
    checkpoints: RwLock<HashMap<String, SessionCheckpoint>>,
}

impl MockPersistence {
    fn storage(&self) -> Arc<MockPersistence> {
        Arc::new(self.clone())
    }
}

// The type is also Clone.
impl Clone for MockPersistence {
    fn clone(&self) -> Self {
        Self {
            checkpoints: RwLock::new(self.checkpoints.read().await.clone()),
        }
    }
}

#[async_trait]
impl PersistenceService for MockPersistence {
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
        Ok(self.checkpoints.read().await.get(session_id).cloned())
    }

    async fn delete_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(
        &self,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, crate::session::persistence::PersistenceError> {
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
        Ok(Vec::new())
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
        _role: crate::session::persistence::AgentRole,
        _idle_minutes: i64,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: crate::session::persistence::AgentRole,
        _purge_after_minutes: i64,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(vec![])
    }
}

// ── Test helpers ─────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 10000,
        dm_scope: DmScope::PerChannelPeer,
    }
}

fn make_message() -> Message {
    Message {
        id: "msg_in".to_string(),
        from: "user_1".to_string(),
        to: "agent_1".to_string(),
        content: "hello".to_string(),
        channel: "test_channel".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    }
}

/// Build a Gateway with a CheckpointManager using the given MockPersistence.
fn make_gw_with_cm(
    config: GatewayConfig,
    sm: Arc<SessionManager>,
    persistence: Arc<MockPersistence>,
) -> crate::gateway::Gateway {
    let cm = Arc::new(CheckpointManager::new(persistence.storage()));
    crate::gateway::Gateway::new(config, sm).with_checkpoint_manager(cm)
}

/// Add session_id to a message using the SessionManager.
async fn add_session(msg: &mut Message, sm: &SessionManager) {
    let sid = sm.find_or_create("test_channel", msg, None).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Test: `with_checkpoint_manager` stores the CheckpointManager.
#[tokio::test]
async fn test_with_checkpoint_manager_stores_cm() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
    ));
    let persistence = Arc::new(MockPersistence::default());
    let cm = Arc::new(CheckpointManager::new(persistence.storage()));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm)).with_checkpoint_manager(cm);

    // The gateway has a CheckpointManager — we cannot inspect it directly (it's private).
    // Verify it is present by confirming send_outbound triggers checkpoint saves
    // (tested in other cases). Here we just verify construction doesn't panic.
    let adapter = Arc::new(MockAdapter::new());
    gw.register_adapter(
        "test_channel".into(),
        Arc::clone(&adapter) as Arc<dyn IMAdapter>,
    )
    .await;
    drop(gw); // clean up
}

/// Test: `with_checkpoint_manager` - gateway works even if no storage is provided.
#[tokio::test]
async fn test_gateway_without_checkpoint_manager_has_no_cm() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
    ));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));

    // Without CheckpointManager, send_outbound should still succeed
    let adapter = Arc::new(MockAdapter::new());
    gw.register_adapter(
        "test_channel".into(),
        Arc::clone(&adapter) as Arc<dyn IMAdapter>,
    )
    .await;

    let mut msg = make_message();
    add_session(&mut msg, &sm).await;

    // Without cm, send_outbound just sends without saving checkpoints
    let result = gw
        .send_outbound("test_channel", &msg.metadata["session_id"], "hello")
        .await;
    assert!(result.is_ok());
}

/// Test: send_outbound with CheckpointManager — checkpoint is saved after successful text send.
#[tokio::test]
async fn test_send_outbound_with_cm_saves_checkpoint_on_success() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
    ));
    let persistence = Arc::new(MockPersistence::default());
    let gw = make_gw_with_cm(config.clone(), Arc::clone(&sm), persistence.clone());

    let adapter = Arc::new(MockAdapter::new());
    gw.register_adapter(
        "test_channel".into(),
        Arc::clone(&adapter) as Arc<dyn IMAdapter>,
    )
    .await;

    let mut msg = make_message();
    add_session(&mut msg, &sm).await;
    let session_id = msg.metadata["session_id"].clone();

    gw.send_outbound("test_channel", &session_id, "hello")
        .await
        .unwrap();

    // Verify: checkpoint was saved
    let cp = persistence
        .checkpoints
        .read()
        .await
        .get(&session_id)
        .cloned();
    assert!(
        cp.is_some(),
        "checkpoint should be saved when cm is configured"
    );
    let cp = cp.unwrap();

    // Verify: pending message was added with sent=true
    assert!(
        !cp.pending_messages.is_empty(),
        "pending message should be added"
    );
    let pending = &cp.pending_messages[0];
    assert!(pending.sent, "pending message should be marked sent=true");
}

/// Test: send_outbound without CheckpointManager — checkpoint is NOT saved.
#[tokio::test]
async fn test_send_outbound_without_cm_does_not_save_checkpoint() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
    ));
    let persistence = Arc::new(MockPersistence::default());

    // Gateway WITHOUT CheckpointManager
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));

    let adapter = Arc::new(MockAdapter::new());
    gw.register_adapter(
        "test_channel".into(),
        Arc::clone(&adapter) as Arc<dyn IMAdapter>,
    )
    .await;

    let mut msg = make_message();
    add_session(&mut msg, &sm).await;
    let session_id = msg.metadata["session_id"].clone();

    gw.send_outbound("test_channel", &session_id, "hello")
        .await
        .unwrap();

    // Verify: no checkpoint was saved
    let cp = persistence
        .checkpoints
        .read()
        .await
        .get(&session_id)
        .cloned();
    assert!(cp.is_none(), "without cm, checkpoint should not be saved");
}

/// Test: send failure does NOT trigger checkpoint save.
#[tokio::test]
async fn test_send_failure_does_not_trigger_checkpoint() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
    ));
    let persistence = Arc::new(MockPersistence::default());
    let gw = make_gw_with_cm(config.clone(), Arc::clone(&sm), persistence.clone());

    let adapter = Arc::new(MockAdapter::new());
    adapter.fail_send = true; // make send_message fail
    gw.register_adapter(
        "test_channel".into(),
        Arc::clone(&adapter) as Arc<dyn IMAdapter>,
    )
    .await;

    let mut msg = make_message();
    add_session(&mut msg, &sm).await;
    let session_id = msg.metadata["session_id"].clone();

    let result = gw.send_outbound("test_channel", &session_id, "hello").await;
    assert!(result.is_err(), "send should fail");

    // Verify: no checkpoint was saved
    let cp = persistence
        .checkpoints
        .read()
        .await
        .get(&session_id)
        .cloned();
    assert!(
        cp.is_none(),
        "on send failure, checkpoint should not be saved"
    );
}

/// Test: interactive message path also triggers checkpoint persistence.
#[tokio::test]
async fn test_send_outbound_interactive_triggers_checkpoint() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
    ));
    let persistence = Arc::new(MockPersistence::default());
    let gw = make_gw_with_cm(config.clone(), Arc::clone(&sm), persistence.clone());

    let adapter = Arc::new(MockAdapter::new());
    gw.register_adapter(
        "test_channel".into(),
        Arc::clone(&adapter) as Arc<dyn IMAdapter>,
    )
    .await;

    let mut msg = make_message();
    add_session(&mut msg, &sm).await;
    let session_id = msg.metadata["session_id"].clone();

    // JSON with msg_type = "interactive"
    let interactive_json = serde_json::json!({
        "msg_type": "interactive",
        "content": r#"{"type":"card","elements":[{"tag":"markdown","content":"**hello**"}]}"#
    })
    .to_string();

    gw.send_outbound("test_channel", &session_id, &interactive_json)
        .await
        .unwrap();

    // Verify: checkpoint was saved
    let cp = persistence
        .checkpoints
        .read()
        .await
        .get(&session_id)
        .cloned();
    assert!(cp.is_some(), "interactive send should also save checkpoint");
    let cp = cp.unwrap();
    assert!(
        !cp.pending_messages.is_empty(),
        "pending message should be added for interactive"
    );
}

/// Test: pending_message is correctly constructed with id and content.
#[tokio::test]
async fn test_send_outbound_pending_message_contains_correct_data() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
    ));
    let persistence = Arc::new(MockPersistence::default());
    let gw = make_gw_with_cm(config.clone(), Arc::clone(&sm), persistence.clone());

    let adapter = Arc::new(MockAdapter::new());
    gw.register_adapter(
        "test_channel".into(),
        Arc::clone(&adapter) as Arc<dyn IMAdapter>,
    )
    .await;

    let mut msg = make_message();
    add_session(&mut msg, &sm).await;
    let session_id = msg.metadata["session_id"].clone();

    gw.send_outbound("test_channel", &session_id, "test content")
        .await
        .unwrap();

    let cp = persistence
        .checkpoints
        .read()
        .await
        .get(&session_id)
        .cloned()
        .unwrap();

    let pending = &cp.pending_messages[0];
    // message id is format "out-{timestamp}"
    assert!(
        pending.message_id.starts_with("out-"),
        "pending message id should start with 'out-', got: {}",
        pending.message_id
    );
    assert_eq!(
        pending.content, "test content",
        "pending message content should match"
    );
    assert!(pending.sent, "pending message should be marked sent");
}
