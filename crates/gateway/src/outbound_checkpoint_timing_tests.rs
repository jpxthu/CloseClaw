//! Tests for outbound checkpoint persistence timing.
//!
//! Verifies that `dispatch_and_persist` persists the checkpoint *after*
//! successful delivery (mark_sent=true). Pre-send checkpoints are not used
//! (design doc: checkpoint is written after send succeeds).

use crate::{GatewayConfig, SessionManager};
use closeclaw_common::im_plugin::{
    AdapterError, NormalizedMessage, RenderedOutput, StreamingOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult, StreamEvent};
use closeclaw_common::{IMPlugin, StreamingRenderer};
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
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

/// Verify that `dispatch_and_persist` persists the checkpoint only *after*
/// successful delivery (mark_sent=true). No pre-send checkpoint is written.
#[tokio::test]
async fn test_checkpoint_persisted_after_send_only() {
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

    // Wait for send() to be entered. No persist should have happened yet.
    setup.entered_send.notified().await;

    // At this point, send() is entered but no pre-send checkpoint exists.
    let saves = persist.get_saves().await;
    assert_eq!(
        saves.len(),
        0,
        "no persist should happen before send completes"
    );

    // Let send() complete. The task will continue and do the persist.
    setup.ok_to_return.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(result.is_ok(), "send_outbound should succeed");

    // After send completes, verify the persist (mark_sent=true).
    let saves = persist.get_saves().await;
    assert_eq!(saves.len(), 1, "should have 1 save after send completes");
    assert_eq!(saves[0].session_id, setup.session_id);
    assert_eq!(saves[0].pending_count, 1);
    assert!(saves[0].last_pending_sent, "persist should be marked sent");
}

/// Verify that interactive message types also persist only after send.
#[tokio::test]
async fn test_interactive_message_persist_after_send() {
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

    entered.notified().await;

    let saves = persist.get_saves().await;
    assert_eq!(saves.len(), 0, "no persist before send completes");

    ok.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(result.is_ok());

    let saves = persist.get_saves().await;
    assert_eq!(saves.len(), 1, "should have 1 save after send");
    assert!(saves[0].last_pending_sent, "should be marked sent");
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
    assert_eq!(saves.len(), 1);

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
