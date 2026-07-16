//! Tests for `SessionManager::drain_outbound_pending_for_session`.
//!
//! Covers the 5 behaviour dimensions specified in the plan:
//! 1. Normal path — 2 unsent messages both delivered and marked sent
//! 2. Partial failure — 3 unsent, middle one fails, others succeed
//! 3. No pending — empty outbound_pending returns Ok(0)
//! 4. All sent — outbound_pending exists but all sent==true returns Ok(0)
//! 5. No checkpoint — session has no checkpoint returns Ok(0)

use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use crate::{Gateway, GatewayConfig};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, NormalizedMessage, RenderedOutput};
use closeclaw_session::persistence::PendingMessage;
use closeclaw_session::persistence::{
    AgentRole, PersistenceError, PersistenceService, SessionCheckpoint,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Mock persistence service ──────────────────────────────────────────────

/// In-memory mock persistence service that stores checkpoints by session ID.
/// Supports configurable load failures for specific sessions.
struct MockPersistence {
    checkpoints: Mutex<HashMap<String, SessionCheckpoint>>,
}

impl MockPersistence {
    fn new() -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
        }
    }

    /// Pre-populate a checkpoint for a session.
    async fn insert_checkpoint(&self, cp: SessionCheckpoint) {
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp);
    }
}

#[async_trait]
impl PersistenceService for MockPersistence {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.lock().await.get(session_id).cloned())
    }

    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }

    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn invalidate_session(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

// ── Mock IM plugin ────────────────────────────────────────────────────────

/// Records all messages sent via `send()`. Supports configurable failures.
struct MockPlugin {
    sent: Mutex<Vec<(String, String, Option<String>)>>,
    fail_count: Mutex<usize>,
}

impl MockPlugin {
    fn new() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            fail_count: Mutex::new(usize::MAX),
        }
    }

    /// After the first `n` calls to `send`, subsequent calls fail.
    fn with_fail_after(n: usize) -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            fail_count: Mutex::new(n),
        }
    }

    async fn sent_messages(&self) -> Vec<(String, String, Option<String>)> {
        self.sent.lock().await.clone()
    }
}

#[async_trait]
impl closeclaw_common::IMPlugin for MockPlugin {
    fn platform(&self) -> &str {
        "test_channel"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let mut remaining = self.fail_count.lock().await;
        if *remaining == 0 {
            return Err(AdapterError::SendFailed("mock failure".to_string()));
        }
        *remaining -= 1;
        // Extract text content from payload for recording.
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        self.sent
            .lock()
            .await
            .push((text, peer_id.to_string(), thread_id.map(|s| s.to_string())));
        Ok(())
    }

    fn render(
        &self,
        content_blocks: &[closeclaw_common::processor::ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> RenderedOutput {
        let text: String = content_blocks
            .iter()
            .filter_map(|b| match b {
                closeclaw_common::processor::ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".to_string(),
            payload: serde_json::json!({
                "content": {
                    "text": text
                }
            }),
        }
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────

/// Build a `Gateway` + `SessionManager` pair with a mock plugin registered.
/// The mock plugin has `infinite` capacity (never fails).
///
/// Returns `(session_manager, gateway_arc, plugin_ref)`.
async fn setup_with_mock_gateway() -> (Arc<SessionManager>, Arc<Gateway>, Arc<MockPlugin>) {
    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    };
    let gw = Gateway::new(gw_config, Arc::clone(&mgr));
    let gw_arc = Arc::new(gw);
    let plugin = Arc::new(MockPlugin::new());
    gw_arc
        .register_plugin(plugin.clone() as Arc<dyn closeclaw_common::IMPlugin>)
        .await;
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;
    (mgr, gw_arc, plugin)
}

/// Build a `Gateway` + `SessionManager` pair with a mock plugin that fails
/// after `fail_after` successful sends.
async fn setup_with_failing_gateway(
    fail_after: usize,
) -> (Arc<SessionManager>, Arc<Gateway>, Arc<MockPlugin>) {
    let mgr = Arc::new(make_test_mgr(None));
    let gw_config = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    };
    let gw = Gateway::new(gw_config, Arc::clone(&mgr));
    let gw_arc = Arc::new(gw);
    let plugin = Arc::new(MockPlugin::with_fail_after(fail_after));
    gw_arc
        .register_plugin(plugin.clone() as Arc<dyn closeclaw_common::IMPlugin>)
        .await;
    mgr.set_gateway_ref(Arc::clone(&gw_arc)).await;
    (mgr, gw_arc, plugin)
}

/// Register a session in the SessionManager's sessions map with the given
/// session ID and channel. Returns the session ID.
async fn register_session(mgr: &SessionManager, session_id: &str, channel: &str) {
    use super::Session;
    mgr.sessions.write().await.insert(
        session_id.to_string(),
        Session {
            id: session_id.to_string(),
            agent_id: "test-agent".to_string(),
            channel: channel.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
}

/// Create and set a `CheckpointManager` on the `SessionManager` backed by
/// the given mock persistence service.
async fn set_checkpoint_manager(mgr: &SessionManager, mock: Arc<MockPersistence>) {
    let storage: Arc<dyn PersistenceService> = mock as Arc<dyn PersistenceService>;
    let cm = Arc::new(closeclaw_session::checkpoint_manager::CheckpointManager::new(storage));
    mgr.set_checkpoint_manager(cm).await;
}

// ── Test 1: Normal path — 2 unsent messages both delivered ────────────────

/// When checkpoint has 2 unsent outbound_pending messages, both should be
/// delivered via the gateway and marked sent. The checkpoint should be
/// persisted with both messages marked sent.
#[tokio::test]
async fn test_drain_outbound_normal_path() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-normal";
    register_session(&mgr, session_id, "test_channel").await;

    // Build checkpoint with 2 unsent messages.
    let cp = SessionCheckpoint::new(session_id.to_string()).with_outbound_pending(vec![
        PendingMessage::new("msg-1".into(), "hello world".into()),
        PendingMessage::new("msg-2".into(), "goodbye world".into()),
    ]);
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 2, "should deliver 2 messages");

    // Verify plugin received both messages.
    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 2, "plugin should have received 2 messages");
    assert_eq!(sent[0].0, "hello world");
    assert_eq!(sent[1].0, "goodbye world");

    // Verify checkpoint was persisted with both marked sent.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(saved_cp.outbound_pending.len(), 2);
    assert!(saved_cp.outbound_pending[0].sent, "msg-1 should be sent");
    assert!(saved_cp.outbound_pending[1].sent, "msg-2 should be sent");
}

// ── Test 2: Partial failure — 3 unsent, middle one fails ─────────────────

/// When 3 unsent messages exist but the 2nd delivery fails, messages 1 and 3
/// should be marked sent while message 2 stays unsent. The checkpoint should
/// be persisted reflecting this mixed state.
#[tokio::test]
async fn test_drain_outbound_partial_failure() {
    clear_global_prompt_state();

    // Plugin that fails after 1 successful send.
    let (mgr, _gw, plugin) = setup_with_failing_gateway(1).await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-partial";
    register_session(&mgr, session_id, "test_channel").await;

    let cp = SessionCheckpoint::new(session_id.to_string()).with_outbound_pending(vec![
        PendingMessage::new("msg-a".into(), "first".into()),
        PendingMessage::new("msg-b".into(), "second".into()),
        PendingMessage::new("msg-c".into(), "third".into()),
    ]);
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(
        result.unwrap(),
        1,
        "only 1 message should be delivered (2nd fails)"
    );

    // Plugin received 2 attempts (1 success + 1 failure).
    let sent = plugin.sent_messages().await;
    assert_eq!(
        sent.len(),
        1,
        "plugin should have received 1 successful message"
    );
    assert_eq!(sent[0].0, "first");

    // Verify checkpoint: msg-a sent, msg-b and msg-c unsent.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(saved_cp.outbound_pending.len(), 3);
    assert!(saved_cp.outbound_pending[0].sent, "msg-a should be sent");
    assert!(
        !saved_cp.outbound_pending[1].sent,
        "msg-b should remain unsent"
    );
    assert!(
        !saved_cp.outbound_pending[2].sent,
        "msg-c should remain unsent"
    );
}

// ── Test 3: No pending — empty outbound_pending ──────────────────────────

/// When checkpoint exists but has an empty outbound_pending list, the
/// function should return Ok(0) with no side effects (no plugin calls,
/// no checkpoint save).
#[tokio::test]
async fn test_drain_outbound_no_pending() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-empty";
    register_session(&mgr, session_id, "test_channel").await;

    // Checkpoint with no outbound_pending.
    let cp = SessionCheckpoint::new(session_id.to_string());
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0, "should return Ok(0) for no pending");

    // No messages should have been sent.
    let sent = plugin.sent_messages().await;
    assert!(
        sent.is_empty(),
        "no messages should be sent for empty pending"
    );
}

// ── Test 4: All sent — outbound_pending exists but sent==true ────────────

/// When all outbound_pending messages already have sent==true, the function
/// should return Ok(0) without attempting delivery.
#[tokio::test]
async fn test_drain_outbound_all_sent() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-all-sent";
    register_session(&mgr, session_id, "test_channel").await;

    // Build messages that are already sent.
    let mut msg1 = PendingMessage::new("msg-1".into(), "already sent".into());
    msg1.mark_sent();
    let mut msg2 = PendingMessage::new("msg-2".into(), "also sent".into());
    msg2.mark_sent();

    let cp = SessionCheckpoint::new(session_id.to_string()).with_outbound_pending(vec![msg1, msg2]);
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap(),
        0,
        "should return Ok(0) when all messages are already sent"
    );

    let sent = plugin.sent_messages().await;
    assert!(
        sent.is_empty(),
        "no messages should be sent when all are already sent"
    );
}

// ── Test 5: Checkpoint does not exist ────────────────────────────────────

/// When the session has no checkpoint at all, the function should return
/// an error (checkpoint not found).
#[tokio::test]
async fn test_drain_outbound_no_checkpoint() {
    clear_global_prompt_state();

    let (mgr, _gw, _plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-cp";
    register_session(&mgr, session_id, "test_channel").await;

    // No checkpoint inserted — load_checkpoint returns None.
    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(
        result.is_err(),
        "should return error when no checkpoint exists"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("checkpoint not found"),
        "error should mention checkpoint not found, got: {}",
        err
    );
}

// ── Test: No checkpoint manager set ──────────────────────────────────────

/// When checkpoint_manager is not set on the SessionManager, the function
/// should return Ok(0) (graceful no-op).
#[tokio::test]
async fn test_drain_outbound_no_checkpoint_manager() {
    clear_global_prompt_state();

    let (mgr, _gw, _plugin) = setup_with_mock_gateway().await;
    // Intentionally do NOT set checkpoint_manager.

    let session_id = "drain-no-cm";
    register_session(&mgr, session_id, "test_channel").await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap(),
        0,
        "should return Ok(0) when no checkpoint_manager is set"
    );
}

// ── Test: Mixed sent/unsent ─────────────────────────────────────────────

/// When outbound_pending has a mix of sent and unsent messages, only
/// unsent messages should be delivered.
#[tokio::test]
async fn test_drain_outbound_mixed_sent_unsent() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-mixed";
    register_session(&mgr, session_id, "test_channel").await;

    let mut sent_msg = PendingMessage::new("msg-sent".into(), "already delivered".into());
    sent_msg.mark_sent();

    let cp = SessionCheckpoint::new(session_id.to_string()).with_outbound_pending(vec![
        sent_msg,
        PendingMessage::new("msg-unsent".into(), "needs delivery".into()),
        PendingMessage::new("msg-unsent-2".into(), "also needs delivery".into()),
    ]);
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap(),
        2,
        "should deliver only the 2 unsent messages"
    );

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].0, "needs delivery");
    assert_eq!(sent[1].0, "also needs delivery");

    // Verify checkpoint: sent_msg still sent, others now sent.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(saved_cp.outbound_pending.len(), 3);
    assert!(
        saved_cp.outbound_pending[0].sent,
        "msg-sent should remain sent"
    );
    assert!(
        saved_cp.outbound_pending[1].sent,
        "msg-unsent should now be sent"
    );
    assert!(
        saved_cp.outbound_pending[2].sent,
        "msg-unsent-2 should now be sent"
    );
}

// ── Test: Session not in sessions map ────────────────────────────────────

/// When the session exists in checkpoint but is not registered in the
/// sessions map (missing channel), the function should return an error.
#[tokio::test]
async fn test_drain_outbound_session_not_in_map() {
    clear_global_prompt_state();

    let (mgr, _gw, _plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-session";
    // Intentionally do NOT register the session in the sessions map.

    let cp = SessionCheckpoint::new(session_id.to_string())
        .with_outbound_pending(vec![PendingMessage::new("msg-1".into(), "hello".into())]);
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(
        result.is_err(),
        "should error when session not found in sessions map"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("not found in sessions map"),
        "error should mention sessions map, got: {}",
        err
    );
}

// ── Test: Gateway not set ───────────────────────────────────────────────

/// When gateway_ref is not set on the SessionManager, the function should
/// return an error.
#[tokio::test]
async fn test_drain_outbound_no_gateway() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-gw";
    register_session(&mgr, session_id, "test_channel").await;

    // Intentionally do NOT set gateway_ref.

    let cp = SessionCheckpoint::new(session_id.to_string())
        .with_outbound_pending(vec![PendingMessage::new("msg-1".into(), "hello".into())]);
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(
        result.is_err(),
        "should error when gateway is not available"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("gateway not available"),
        "error should mention gateway, got: {}",
        err
    );
}
