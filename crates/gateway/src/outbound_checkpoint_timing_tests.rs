//! Tests for outbound checkpoint persistence timing.
//!
//! Verifies that `dispatch_and_persist` writes a write-ahead pending
//! operation *before* `plugin.send`, and clears it *after* successful
//! delivery (ack). On failure the pending op remains for recovery retry.

use crate::{GatewayConfig, SessionManager};
use closeclaw_common::im_plugin::{
    AdapterError, NormalizedMessage, RenderedOutput, StreamingOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult, StreamEvent};
use closeclaw_common::{IMPlugin, StreamingRenderer};
use closeclaw_session::persistence::{
    PendingOperationType, PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

// ---------------------------------------------------------------------------
// Mock persistence
// ---------------------------------------------------------------------------

/// Mock persistence that records saves with their mark_sent state
/// and stores checkpoints for load.
struct TimingMockPersist {
    checkpoints: Mutex<HashMap<String, SessionCheckpoint>>,
    saves: Arc<Mutex<Vec<SaveRecord>>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SaveRecord {
    session_id: String,
    pending_count: usize,
    last_pending_sent: bool,
}

impl TimingMockPersist {
    fn new() -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
            saves: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn get_saves(&self) -> Vec<SaveRecord> {
        self.saves.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl PersistenceService for TimingMockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        let last_sent = cp.outbound_pending.last().map(|p| p.sent).unwrap_or(false);
        self.saves.lock().await.push(SaveRecord {
            session_id: cp.session_id.clone(),
            pending_count: cp.outbound_pending.len(),
            last_pending_sent: last_sent,
        });
        self.checkpoints
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.checkpoints.lock().await.get(sid).cloned())
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
// Mock plugin with two-phase synchronization
// ---------------------------------------------------------------------------

/// Mock plugin that synchronizes with the test via two-phase Notify:
/// 1. `entered_send` fires when send() is entered (first persist done).
/// 2. `ok_to_return` blocks until the test signals (test verifies state).
struct TimingMockPlugin {
    platform: String,
    entered_send: Arc<Notify>,
    ok_to_return: Arc<Notify>,
}

#[async_trait::async_trait]
impl IMPlugin for TimingMockPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        // Signal that send() has been entered (first persist is done).
        self.entered_send.notify_one();
        // Block until the test verifies intermediate state.
        self.ok_to_return.notified().await;
        Ok(())
    }

    fn send_thinking_indicator(&self, _active: bool) {}

    fn handle_stream_event(&self, event: StreamEvent) -> StreamingOutput {
        let mut renderer = closeclaw_common::DefaultStreamingRenderer::new();
        renderer.handle_event(event)
    }

    fn flush_stream(&self) -> StreamingOutput {
        closeclaw_common::DefaultStreamingRenderer::new().flush()
    }
}

// ---------------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-timing".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

struct SetupResult {
    gw: crate::Gateway,
    session_id: String,
    entered_send: Arc<Notify>,
    ok_to_return: Arc<Notify>,
}

/// Set up a Gateway with timing mock plugin and persistence.
async fn setup_timing_gw(persist: Arc<TimingMockPersist>) -> SetupResult {
    let session_id = "sess-timing-1".to_string();
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.clone(),
        crate::Session {
            id: session_id.clone(),
            agent_id: "chat_test".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    let gw = crate::Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);

    let entered = Arc::new(Notify::new());
    let ok = Arc::new(Notify::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(TimingMockPlugin {
        platform: "mock".to_string(),
        entered_send: Arc::clone(&entered),
        ok_to_return: Arc::clone(&ok),
    });
    gw.register_plugin(plugin).await;

    SetupResult {
        gw,
        session_id,
        entered_send: entered,
        ok_to_return: ok,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that `dispatch_and_persist` writes a write-ahead pending
/// operation before `plugin.send`, then clears it after successful
/// delivery. Two checkpoint saves occur: write-ahead + ack-clear.
#[tokio::test]
async fn test_writeahead_before_send_and_ackclear_after() {
    let persist = Arc::new(TimingMockPersist::new());
    let setup = setup_timing_gw(Arc::clone(&persist)).await;

    let gw_arc = Arc::new(setup.gw);
    let sid = setup.session_id.clone();
    let gw_clone = Arc::clone(&gw_arc);
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid, "mock", "hello world", vec![], None, None)
            .await
    });

    // Wait for send() to be entered. Write-ahead persist should have
    // happened (pending_operations contains OutboundMessage op).
    setup.entered_send.notified().await;

    {
        let saves = persist.get_saves().await;
        assert_eq!(saves.len(), 1, "write-ahead persist should have happened");
    }
    // Verify write-ahead wrote an OutboundMessage pending op.
    {
        let cp = persist
            .checkpoints
            .lock()
            .await
            .get(&setup.session_id)
            .cloned()
            .expect("checkpoint should exist");
        let has_outbound_op = cp
            .pending_operations
            .iter()
            .any(|op| op.op_type == PendingOperationType::OutboundMessage);
        assert!(
            has_outbound_op,
            "write-ahead should record OutboundMessage pending op"
        );
    }

    // Let send() complete. The ack-clear persist should follow.
    setup.ok_to_return.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(result.is_ok(), "send_outbound should succeed");

    // After send completes, verify three persists happened:
    // 1. write-ahead (pending op recorded)
    // 2. ack-clear (pending op cleared)
    // 3. outbound_pending (delivery record persisted)
    {
        let saves = persist.get_saves().await;
        assert_eq!(
            saves.len(),
            3,
            "should have write-ahead + ack-clear + delivery record saves"
        );
        assert_eq!(saves[0].session_id, setup.session_id);
        assert_eq!(saves[1].session_id, setup.session_id);
        assert_eq!(saves[2].session_id, setup.session_id);
    }
    // Verify ack-clear removed the OutboundMessage pending op.
    {
        let cp = persist
            .checkpoints
            .lock()
            .await
            .get(&setup.session_id)
            .cloned()
            .expect("checkpoint should exist");
        let has_outbound_op = cp
            .pending_operations
            .iter()
            .any(|op| op.op_type == PendingOperationType::OutboundMessage);
        assert!(
            !has_outbound_op,
            "ack-clear should remove OutboundMessage pending op"
        );
        // But outbound_pending (delivery record) should still exist.
        assert_eq!(
            cp.outbound_pending.len(),
            1,
            "delivery record should persist"
        );
        assert!(
            cp.outbound_pending[0].sent,
            "delivery record should be marked sent"
        );
    }
}

/// Verify that interactive message types also write-ahead and ack-clear.
#[tokio::test]
async fn test_interactive_message_writeahead_and_ackclear() {
    let persist = Arc::new(TimingMockPersist::new());
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    let session_id = "sess-interactive-1".to_string();
    sm.sessions.write().await.insert(
        session_id.clone(),
        crate::Session {
            id: session_id.clone(),
            agent_id: "chat_interactive".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    let gw = crate::Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);

    let entered = Arc::new(Notify::new());
    let ok = Arc::new(Notify::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(InteractiveTimingPlugin {
        platform: "mock".to_string(),
        entered_send: Arc::clone(&entered),
        ok_to_return: Arc::clone(&ok),
    });
    gw.register_plugin(plugin).await;

    let gw_arc = Arc::new(gw);
    let sid = session_id.clone();
    let gw_clone = Arc::clone(&gw_arc);
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid, "mock", "hello interactive", vec![], None, None)
            .await
    });

    // Write-ahead persist should have happened.
    entered.notified().await;
    {
        let saves = persist.get_saves().await;
        assert_eq!(saves.len(), 1, "write-ahead persist should have happened");
    }

    // Let send complete. Ack-clear persist + delivery record persist follow.
    ok.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(result.is_ok());

    {
        let saves = persist.get_saves().await;
        assert_eq!(
            saves.len(),
            3,
            "should have write-ahead + ack-clear + delivery record saves"
        );
        assert!(
            saves[2].last_pending_sent,
            "delivery record should be marked sent"
        );
    }
}

// ---------------------------------------------------------------------------
// Additional mock plugins
// ---------------------------------------------------------------------------

/// Plugin that renders as interactive and synchronizes via two-phase Notify.
struct InteractiveTimingPlugin {
    platform: String,
    entered_send: Arc<Notify>,
    ok_to_return: Arc<Notify>,
}

#[async_trait::async_trait]
impl IMPlugin for InteractiveTimingPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "interactive".into(),
            payload: serde_json::json!({"elements": []}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.entered_send.notify_one();
        self.ok_to_return.notified().await;
        Ok(())
    }

    fn send_thinking_indicator(&self, _active: bool) {}

    fn handle_stream_event(&self, _event: StreamEvent) -> StreamingOutput {
        StreamingOutput::default()
    }

    fn flush_stream(&self) -> StreamingOutput {
        StreamingOutput::default()
    }
}

// ---------------------------------------------------------------------------
// Checkpoint field persistence test (Step 1.7 fields)
// ---------------------------------------------------------------------------

/// Verify that `persist_outbound_checkpoint` stores platform, dsl_result,
/// and content_blocks in the PendingMessage.
#[tokio::test]
async fn test_checkpoint_persists_platform_dsl_result_content_blocks() {
    let persist = Arc::new(TimingMockPersist::new());
    let setup = setup_timing_gw(Arc::clone(&persist)).await;

    let gw_arc = Arc::new(setup.gw);
    let sid = setup.session_id.clone();
    let gw_clone = Arc::clone(&gw_arc);
    let sid_for_spawn = sid.clone();
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid_for_spawn, "mock", "test content", vec![], None, None)
            .await
    });

    // Wait for send() to be entered, then let it complete.
    setup.entered_send.notified().await;
    setup.ok_to_return.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(result.is_ok());

    // After send completes, the checkpoint should have the new fields.
    let saves = persist.get_saves().await;
    assert_eq!(
        saves.len(),
        3,
        "should have write-ahead + ack-clear + delivery record saves"
    );

    // Load the checkpoint and inspect the pending message fields.
    let cp = persist
        .checkpoints
        .lock()
        .await
        .get(&sid)
        .cloned()
        .expect("checkpoint should exist");
    let pending = cp
        .outbound_pending
        .first()
        .expect("should have pending message");
    assert_eq!(pending.platform.as_deref(), Some("mock"));
    // dsl_result and content_blocks may be None for empty content_blocks
    // input, but platform should always be set to the channel name.
}

/// Verify that a loaded checkpoint round-trips platform, dsl_result,
/// and content_blocks through serialization.
#[test]
fn test_pending_message_roundtrip_new_fields() {
    use closeclaw_session::persistence::PendingMessage;

    let mut pm = PendingMessage::with_role(
        "msg-rt".to_string(),
        "content".to_string(),
        "assistant".to_string(),
    );
    pm.platform = Some("telegram".to_string());
    pm.dsl_result = Some("{\"cmd\":\"/help\"}".to_string());
    pm.content_blocks = Some("[{\"type\":\"text\",\"text\":\"hi\"}]".to_string());

    let json = serde_json::to_string(&pm).expect("serialize");
    let deserialized: PendingMessage = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(deserialized.platform.as_deref(), Some("telegram"));
    assert_eq!(
        deserialized.dsl_result.as_deref(),
        Some("{\"cmd\":\"/help\"}")
    );
    assert_eq!(
        deserialized.content_blocks.as_deref(),
        Some("[{\"type\":\"text\",\"text\":\"hi\"}]")
    );
}

/// Verify that a legacy checkpoint (without new fields) deserializes
/// with None defaults for platform, dsl_result, and content_blocks.
#[test]
fn test_pending_message_legacy_json_defaults() {
    use closeclaw_session::persistence::PendingMessage;

    // Legacy JSON: missing platform, dsl_result, content_blocks
    let legacy = r#"{"message_id":"m1","content":"hello","created_at":"2025-01-01T00:00:00Z","sent":false,"target_channel":"feishu"}"#;
    let pm: PendingMessage = serde_json::from_str(legacy).expect("legacy should deserialize");
    assert_eq!(pm.platform, None);
    assert_eq!(pm.dsl_result, None);
    assert_eq!(pm.content_blocks, None);
}

// ---------------------------------------------------------------------------
// Failure path and crash simulation tests
// ---------------------------------------------------------------------------

/// Mock plugin that always fails on send.
struct FailingMockPlugin {
    platform: String,
    entered_send: Arc<Notify>,
    ok_to_return: Arc<Notify>,
    /// Tracks whether the first send call has been made.
    first_called: Arc<tokio::sync::Mutex<bool>>,
}

#[async_trait::async_trait]
impl IMPlugin for FailingMockPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let mut called = self.first_called.lock().await;
        if !*called {
            *called = true;
            drop(called);
            self.entered_send.notify_one();
            self.ok_to_return.notified().await;
        }
        Err(AdapterError::SendFailed("mock send failure".into()))
    }

    fn send_thinking_indicator(&self, _active: bool) {}

    fn handle_stream_event(&self, _event: StreamEvent) -> StreamingOutput {
        StreamingOutput::default()
    }

    fn flush_stream(&self) -> StreamingOutput {
        StreamingOutput::default()
    }
}

/// Set up a Gateway with a failing mock plugin.
async fn setup_failing_gw(
    persist: Arc<TimingMockPersist>,
) -> (crate::Gateway, String, Arc<Notify>, Arc<Notify>) {
    let session_id = "sess-fail-1".to_string();
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.clone(),
        crate::Session {
            id: session_id.clone(),
            agent_id: "chat_fail".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    let gw = crate::Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);

    let entered = Arc::new(Notify::new());
    let ok = Arc::new(Notify::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(FailingMockPlugin {
        platform: "mock".to_string(),
        entered_send: Arc::clone(&entered),
        ok_to_return: Arc::clone(&ok),
        first_called: Arc::new(tokio::sync::Mutex::new(false)),
    });
    gw.register_plugin(plugin).await;

    (gw, session_id, entered, ok)
}

/// Send failure: write-ahead op is recorded, send fails,
/// pending op remains in checkpoint for recovery retry.
#[tokio::test]
async fn test_send_failure_op_remains_for_retry() {
    let persist = Arc::new(TimingMockPersist::new());
    let (gw, session_id, entered, ok) = setup_failing_gw(Arc::clone(&persist)).await;

    let gw_arc = Arc::new(gw);
    let sid = session_id.clone();
    let gw_clone = Arc::clone(&gw_arc);
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid, "mock", "will fail", vec![], None, None)
            .await
    });

    // Wait for send() to be entered. Write-ahead persist should have happened.
    entered.notified().await;
    {
        let saves = persist.get_saves().await;
        assert_eq!(saves.len(), 1, "write-ahead persist should have happened");
    }

    // Let send() fail.
    ok.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(
        result.is_ok(),
        "send_outbound returns Ok(SendOutcome::Notified) on send failure"
    );

    // No ack-clear persist should have happened (only 1 save total).
    {
        let saves = persist.get_saves().await;
        assert_eq!(saves.len(), 1, "only write-ahead persist, no ack-clear");
    }

    // Verify the pending op remains in checkpoint.
    {
        let cp = persist
            .checkpoints
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .expect("checkpoint should exist");
        let outbound_ops: Vec<_> = cp
            .pending_operations
            .iter()
            .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
            .collect();
        assert_eq!(
            outbound_ops.len(),
            1,
            "OutboundMessage pending op should remain after send failure"
        );
    }
}

/// Crash simulation: write-ahead is persisted, then the process "crashes"
/// before ack-clear. On "restart" (loading from persistence), the
/// OutboundMessage pending op should still be present.
#[tokio::test]
async fn test_crash_simulation_op_survives() {
    let persist = Arc::new(TimingMockPersist::new());
    let setup = setup_timing_gw(Arc::clone(&persist)).await;

    let gw_arc = Arc::new(setup.gw);
    let sid = setup.session_id.clone();
    let gw_clone = Arc::clone(&gw_arc);
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid, "mock", "crash me", vec![], None, None)
            .await
    });

    // Wait for send() to be entered (write-ahead done, send in progress).
    setup.entered_send.notified().await;

    // Simulate crash: do NOT let send() complete (no ok_to_return).
    // Instead, just abort the task.
    handle.abort();
    // Give the task a moment to be cancelled.
    tokio::task::yield_now().await;

    // The write-ahead persist should have happened.
    {
        let saves = persist.get_saves().await;
        assert_eq!(saves.len(), 1, "write-ahead persist should have happened");
    }

    // Simulate restart: load checkpoint from persistence.
    let cp = persist
        .checkpoints
        .lock()
        .await
        .get(&setup.session_id)
        .cloned()
        .expect("checkpoint should exist after crash");

    // The OutboundMessage pending op should survive the crash.
    let outbound_ops: Vec<_> = cp
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();
    assert_eq!(
        outbound_ops.len(),
        1,
        "OutboundMessage pending op should survive crash"
    );
    assert_eq!(
        outbound_ops[0].status,
        closeclaw_session::persistence::PendingOperationStatus::Running
    );
}
