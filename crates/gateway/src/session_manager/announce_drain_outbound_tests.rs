//! Tests for `SessionManager::drain_outbound_pending_for_session`.
//!
//! Step 1.3: Drive source is `pending_operations` with `op_type == OutboundMessage`.
//!
//! Behaviour dimensions:
//! 1. Normal path — OutboundMessage op → cache hit → delivered → op cleared
//! 2. Cache miss — transcript fallback → delivered → op cleared
//! 3. Partial failure — some ops delivered, some failed → failed ops preserved
//! 4. No pending ops — empty pending_operations returns Ok(0)
//! 5. No checkpoint — session has no checkpoint returns error
//! 6. No gateway — returns error
//! 7. Op with no content source — skipped, op preserved
//! 8. target_channel over session fallback
//! 9. Empty target_channel falls back to session channel
//! 10. Mixed op types — only OutboundMessage ops are processed
//! 11. All ops delivered → pending_operations empty
//! 12. Op preserved on Err (not Notified) → retry on next startup

use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use crate::{Gateway, GatewayConfig};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, NormalizedMessage, RenderedOutput};
use closeclaw_session::pending_operation_detail::PendingOperationDetail;
use closeclaw_session::persistence::{
    AgentRole, PendingOperation, PendingOperationStatus, PendingOperationType, PersistenceError,
    PersistenceService, SessionCheckpoint,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Mock persistence service ──────────────────────────────────────────────

struct MockPersistence {
    checkpoints: Mutex<HashMap<String, SessionCheckpoint>>,
}

impl MockPersistence {
    fn new() -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
        }
    }

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
        self: &Self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_expired_archived_sessions_for_agent(
        self: &Self,
        _: &str,
        _: AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

// ── Mock IM plugin ────────────────────────────────────────────────────────

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
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let mut remaining = self.fail_count.lock().await;
        if *remaining == 0 {
            return Err(AdapterError::SendFailed("mock failure".to_string()));
        }
        *remaining -= 1;
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

async fn set_checkpoint_manager(mgr: &SessionManager, mock: Arc<MockPersistence>) {
    let storage: Arc<dyn PersistenceService> = mock as Arc<dyn PersistenceService>;
    let cm = Arc::new(closeclaw_session::checkpoint_manager::CheckpointManager::new(storage));
    mgr.set_checkpoint_manager(cm).await;
}

/// Build an OutboundMessage pending operation with the given parameters.
fn make_outbound_op(message_id: &str, target_channel: &str) -> PendingOperation {
    PendingOperation {
        op_id: message_id.into(),
        op_type: PendingOperationType::OutboundMessage,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::OutboundMessage {
            target_channel: target_channel.into(),
            message_id: message_id.into(),
            delivery_status: "pending".into(),
        },
        created_at: chrono::Utc::now(),
    }
}

/// Build a checkpoint with an OutboundMessage pending op and a matching
/// outbound_pending cache entry.
fn make_cp_with_op_and_cache(
    session_id: &str,
    message_id: &str,
    target_channel: &str,
    content: &str,
) -> SessionCheckpoint {
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op(message_id, target_channel));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            message_id.into(),
            content.into(),
            target_channel.into(),
        ));
    cp
}

// ── Test 1: Normal path — op → cache hit → delivered → op cleared ────────

#[tokio::test]
async fn test_drain_outbound_normal_path() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-normal";
    register_session(&mgr, session_id, "other_channel").await;

    let cp = make_cp_with_op_and_cache(session_id, "msg-1", "test_channel", "hello world");
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 1, "should deliver 1 message");

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 1, "plugin should have received 1 message");
    assert_eq!(sent[0].0, "hello world");

    // Verify the OutboundMessage op was cleared from pending_operations.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    assert!(
        saved_cp
            .pending_operations
            .iter()
            .all(|op| op.op_type != PendingOperationType::OutboundMessage),
        "OutboundMessage ops should be cleared after delivery"
    );
}

// ── Test 2: Cache hit — transcript refreshes stale cache content ──────

#[tokio::test]
async fn test_drain_outbound_transcript_fallback() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-transcript";
    register_session(&mgr, session_id, "test_channel").await;

    // Set up ConversationSession with a matching assistant message.
    use closeclaw_common::ContentBlock;
    use closeclaw_session::llm_session::ConversationSession;

    let mut cs = ConversationSession::new(
        session_id.into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );
    cs.append_transcript(
        "assistant",
        vec![ContentBlock::Text("latest transcript content".into())],
    );
    mgr.conversation_sessions.write().await.insert(
        session_id.to_string(),
        Arc::new(tokio::sync::RwLock::new(cs)),
    );

    // Checkpoint with OutboundMessage op AND outbound_pending cache entry.
    // Cache hit → use cached content as transcript key → transcript refreshes
    // to the latest version (transcript is authoritative source).
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("msg-t1", "test_channel"));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "msg-t1".into(),
            "latest transcript content".into(),
            "test_channel".into(),
        ));
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(
        result.unwrap(),
        1,
        "should deliver 1 message via transcript-refreshed content"
    );

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "latest transcript content");

    // Verify op was cleared.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    assert!(saved_cp
        .pending_operations
        .iter()
        .all(|op| op.op_type != PendingOperationType::OutboundMessage),);
}

// ── Test 3: Partial failure — Sent cleared, Notified preserved for retry ─
// Note: gateway dispatch_and_persist wraps plugin send failures as Ok(Notified).
// So "partial failure" means: 1 successful (Sent), 2 failed (Notified).
// Sent ops are cleared; Notified ops are preserved for next retry.

#[tokio::test]
async fn test_drain_outbound_partial_failure() {
    clear_global_prompt_state();

    // Plugin that fails after 1 successful send.
    let (mgr, _gw, plugin) = setup_with_failing_gateway(1).await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-partial";
    register_session(&mgr, session_id, "test_channel").await;

    let mut cp = SessionCheckpoint::new(session_id.into());
    // 3 OutboundMessage ops.
    cp.record_outbound_pending_op(make_outbound_op("msg-a", "test_channel"));
    cp.record_outbound_pending_op(make_outbound_op("msg-b", "test_channel"));
    cp.record_outbound_pending_op(make_outbound_op("msg-c", "test_channel"));
    // Cache entries for all 3.
    for (id, content) in [("msg-a", "first"), ("msg-b", "second"), ("msg-c", "third")] {
        cp.outbound_pending
            .push(closeclaw_common::PendingMessage::with_target_channel(
                id.into(),
                content.into(),
                "test_channel".into(),
            ));
    }
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    // msg-a Sent, msg-b and msg-c Notified (gateway wraps send failures).
    // delivered count only includes Sent.
    assert_eq!(
        result.unwrap(),
        1,
        "only the successful message should count"
    );

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "first");

    // Only msg-a (Sent) should be cleared; msg-b and msg-c (Notified) preserved.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    let remaining_ops: Vec<_> = saved_cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();
    assert_eq!(
        remaining_ops.len(),
        2,
        "Notified ops (msg-b, msg-c) should be preserved for retry"
    );
    let remaining_ids: Vec<_> = remaining_ops.iter().map(|op| op.op_id.as_str()).collect();
    assert!(
        remaining_ids.contains(&"msg-b"),
        "msg-b should be preserved"
    );
    assert!(
        remaining_ids.contains(&"msg-c"),
        "msg-c should be preserved"
    );
}

// ── Test 4: No pending ops — empty pending_operations ────────────────────

#[tokio::test]
async fn test_drain_outbound_no_pending_ops() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-empty";
    register_session(&mgr, session_id, "test_channel").await;

    let cp = SessionCheckpoint::new(session_id.into());
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0, "should return Ok(0) for no pending ops");

    let sent = plugin.sent_messages().await;
    assert!(sent.is_empty());
}

// ── Test 5: No checkpoint — returns error ────────────────────────────────

#[tokio::test]
async fn test_drain_outbound_no_checkpoint() {
    clear_global_prompt_state();

    let (mgr, _gw, _plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-cp";
    register_session(&mgr, session_id, "test_channel").await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("checkpoint not found"), "got: {}", err);
}

// ── Test 6: No checkpoint manager set ────────────────────────────────────

#[tokio::test]
async fn test_drain_outbound_no_checkpoint_manager() {
    clear_global_prompt_state();

    let (mgr, _gw, _plugin) = setup_with_mock_gateway().await;

    let session_id = "drain-no-cm";
    register_session(&mgr, session_id, "test_channel").await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

// ── Test 7: Op with no content source — skipped, op preserved ────────────

#[tokio::test]
async fn test_drain_outbound_no_content_source_skipped() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-content";
    register_session(&mgr, session_id, "test_channel").await;

    // OutboundMessage op with no matching cache entry and no transcript.
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("missing-msg", "test_channel"));
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(
        result.is_ok(),
        "should return Ok(0), not error: {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap(),
        0,
        "should deliver 0 when content not found"
    );

    let sent = plugin.sent_messages().await;
    assert!(sent.is_empty());

    // Verify op was NOT cleared (skipped = preserved for retry).
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    let remaining: Vec<_> = saved_cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();
    assert_eq!(remaining.len(), 1, "skipped op should be preserved");
    assert_eq!(remaining[0].op_id, "missing-msg");
}

// ── Test 8: target_channel used over session fallback ────────────────────

#[tokio::test]
async fn test_drain_outbound_uses_target_channel() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-target-ch";
    register_session(&mgr, session_id, "session_channel").await;

    let cp = make_cp_with_op_and_cache(session_id, "msg-1", "test_channel", "target msg");
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 1);

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "target msg");
}

// ── Test 9: Empty target_channel falls back to session channel ───────────

#[tokio::test]
async fn test_drain_outbound_fallback_to_session_channel() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-fallback";
    register_session(&mgr, session_id, "test_channel").await;

    // Op with empty target_channel.
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("msg-1", ""));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::new(
            "msg-1".into(),
            "fallback msg".into(),
        ));
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 1);

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "fallback msg");
}

// ── Test 10: No session in map + empty target_channel → skipped ──────────

#[tokio::test]
async fn test_drain_outbound_no_session_no_channel_skipped() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-sess";
    // Intentionally do NOT register the session.

    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("msg-1", ""));
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::new(
            "msg-1".into(),
            "hello".into(),
        ));
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);

    let sent = plugin.sent_messages().await;
    assert!(sent.is_empty());
}

// ── Test 11: Mixed op types — only OutboundMessage ops processed ────────

#[tokio::test]
async fn test_drain_outbound_mixed_op_types() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-mixed-ops";
    register_session(&mgr, session_id, "test_channel").await;

    let mut cp = SessionCheckpoint::new(session_id.into());
    // ToolCall op — should be ignored.
    cp.pending_operations.push(PendingOperation {
        op_id: "tool_1".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::ToolCall {
            tool_name: "bash".into(),
            args_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });
    // OutboundMessage op — should be processed.
    cp.record_outbound_pending_op(make_outbound_op("msg-1", "test_channel"));
    // SubSessionSpawn op — should be ignored.
    cp.pending_operations.push(PendingOperation {
        op_id: "child_1".into(),
        op_type: PendingOperationType::SubSessionSpawn,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::SubSessionSpawn {
            child_session_id: "child-1".into(),
            agent_id: "eda".into(),
            task_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "msg-1".into(),
            "outbound content".into(),
            "test_channel".into(),
        ));
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(
        result.unwrap(),
        1,
        "only OutboundMessage op should be delivered"
    );

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "outbound content");

    // Verify: OutboundMessage cleared, ToolCall and SubSessionSpawn preserved.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(saved_cp.pending_operations.len(), 2);
    let op_ids: Vec<&str> = saved_cp
        .pending_operations
        .iter()
        .map(|op| op.op_id.as_str())
        .collect();
    assert!(op_ids.contains(&"tool_1"));
    assert!(op_ids.contains(&"child_1"));
    assert!(!op_ids.contains(&"msg-1"));
}

// ── Test 13: Notified path (plugin send failure) → op preserved for retry ─
// Note: gateway dispatch_and_persist wraps plugin send failures as
// Ok(Notified), not Err. Per doc "宁可重复也不遗漏", Notified ops are
// preserved so they can be retried on the next drain cycle.

#[tokio::test]
async fn test_drain_outbound_notified_clears_op_on_failure() {
    clear_global_prompt_state();

    // Plugin that fails immediately — all sends → Notified.
    let (mgr, _gw, plugin) = setup_with_failing_gateway(0).await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-err";
    register_session(&mgr, session_id, "test_channel").await;

    let cp = make_cp_with_op_and_cache(session_id, "msg-1", "test_channel", "will fail");
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    // Plugin send failed → gateway wraps as Notified → op NOT cleared.
    assert_eq!(result.unwrap(), 0, "failed sends not counted as delivered");

    let sent = plugin.sent_messages().await;
    assert!(
        sent.is_empty(),
        "no messages should reach plugin when it fails immediately"
    );

    // Verify op was PRESERVED (Notified = send failed, retry on next drain).
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    let remaining_ops: Vec<_> = saved_cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();
    assert_eq!(
        remaining_ops.len(),
        1,
        "Notified op should be preserved for retry"
    );
    assert_eq!(remaining_ops[0].op_id, "msg-1");
}

// ── Test 15: Gateway not set → error ─────────────────────────────────────

#[tokio::test]
async fn test_drain_outbound_no_gateway() {
    clear_global_prompt_state();

    let mgr = Arc::new(make_test_mgr(None));
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-gw";
    register_session(&mgr, session_id, "test_channel").await;

    let cp = make_cp_with_op_and_cache(session_id, "msg-1", "test_channel", "hello");
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("gateway not available"), "got: {}", err);
}

// ── Test 17: Pending ops preserved across crash simulation ──────────────

/// Simulate crash after write-ahead but before ack: checkpoint should
/// contain the OutboundMessage op and no outbound_pending cache entry
/// for that message (write-ahead is in pending_operations only).
#[tokio::test]
async fn test_drain_outbound_pending_op_survives_crash() {
    clear_global_prompt_state();

    let (mgr, _gw, _plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-crash-sim";
    register_session(&mgr, session_id, "test_channel").await;

    // Simulate: pending_operations has OutboundMessage op, but outbound_pending
    // is empty (write-ahead succeeded, but message was never added to cache
    // because the crash happened before dispatch_and_persist added it).
    let mut cp = SessionCheckpoint::new(session_id.into());
    cp.record_outbound_pending_op(make_outbound_op("msg-crash", "test_channel"));
    // outbound_pending is empty — no cache entry.
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    // Content not found in cache or transcript → op skipped (preserved).
    assert_eq!(result.unwrap(), 0);

    // Verify op is still in pending_operations.
    let saved_cp = mock.load_checkpoint(session_id).await.unwrap().unwrap();
    let remaining: Vec<_> = saved_cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();
    assert_eq!(
        remaining.len(),
        1,
        "op should survive crash (preserved for retry)"
    );
    assert_eq!(remaining[0].op_id, "msg-crash");
}

// ── Test 18: No ConversationSession — fallback to cache ──────────────────

#[tokio::test]
async fn test_drain_outbound_no_conv_session_uses_cache() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-no-conv";
    register_session(&mgr, session_id, "test_channel").await;
    // No ConversationSession registered.

    let cp = make_cp_with_op_and_cache(session_id, "msg-1", "test_channel", "cached content");
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok(), "drain should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 1);

    let sent = plugin.sent_messages().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "cached content");
}

// ── Test 19: Cache entries without matching ops → ignored ────────────────
//
// outbound_pending cache entries are a content store, NOT the drive source.
// When cache has entries but no matching OutboundMessage ops exist in
// pending_operations, drain should not process them.

#[tokio::test]
async fn test_drain_outbound_cache_entries_no_ops_ignored() {
    clear_global_prompt_state();

    let (mgr, _gw, plugin) = setup_with_mock_gateway().await;
    let mock = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr, mock.clone()).await;

    let session_id = "drain-cache-no-ops";
    register_session(&mgr, session_id, "test_channel").await;

    // Checkpoint has cache entries but NO OutboundMessage ops.
    let mut cp = SessionCheckpoint::new(session_id.into());
    // Cache entry exists (content stored).
    cp.outbound_pending
        .push(closeclaw_common::PendingMessage::with_target_channel(
            "orphan-msg".into(),
            "cached content".into(),
            "test_channel".into(),
        ));
    // No record_outbound_pending_op call — no matching op.
    mock.insert_checkpoint(cp).await;

    let result = mgr.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0, "no ops → no delivery");

    let sent = plugin.sent_messages().await;
    assert!(
        sent.is_empty(),
        "cache entries without ops should not be delivered"
    );
}

// ── Test 20: Repeated restart — same message not re-delivered ───────────
//
// "宁可重复也不遗漏" means no dedup during write, but once an op is
// cleared after successful delivery, a subsequent restart should NOT
// re-deliver the same message. The op is gone from pending_operations.

#[tokio::test]
async fn test_drain_outbound_repeated_restart_no_redelivery() {
    clear_global_prompt_state();

    // --- First restart: op present → delivered → op cleared ---
    let (mgr1, _gw1, _plugin1) = setup_with_mock_gateway().await;
    let mock1 = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr1, mock1.clone()).await;

    let session_id = "drain-repeat";
    register_session(&mgr1, session_id, "test_channel").await;

    let cp1 = make_cp_with_op_and_cache(session_id, "msg-repeat", "test_channel", "repeat content");
    mock1.insert_checkpoint(cp1).await;

    let result = mgr1.drain_outbound_pending_for_session(session_id).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1, "first drain delivers 1 message");

    // Verify op was cleared.
    let cp_after = mock1.load_checkpoint(session_id).await.unwrap().unwrap();
    assert!(
        cp_after
            .pending_operations
            .iter()
            .all(|op| op.op_type != PendingOperationType::OutboundMessage),
        "op should be cleared after first drain"
    );

    // --- Second restart: same checkpoint (op already cleared) ---
    // In real daemon restart, the checkpoint is loaded from persistence.
    // Since op was cleared in step 1, the loaded checkpoint has no
    // OutboundMessage ops → drain returns Ok(0).
    let (mgr2, _gw2, plugin2) = setup_with_mock_gateway().await;
    let mock2 = Arc::new(MockPersistence::new());
    set_checkpoint_manager(&mgr2, mock2.clone()).await;
    register_session(&mgr2, session_id, "test_channel").await;

    // Load the checkpoint that was saved after first drain (op cleared).
    let cp2 = mock1.load_checkpoint(session_id).await.unwrap().unwrap();
    mock2.insert_checkpoint(cp2).await;

    let result2 = mgr2.drain_outbound_pending_for_session(session_id).await;
    assert!(result2.is_ok());
    assert_eq!(
        result2.unwrap(),
        0,
        "second drain delivers nothing (op already cleared)"
    );

    let sent2 = plugin2.sent_messages().await;
    assert!(
        sent2.is_empty(),
        "message should NOT be re-delivered after op was cleared"
    );
}
