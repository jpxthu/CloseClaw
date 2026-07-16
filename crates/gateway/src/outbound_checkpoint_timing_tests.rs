//! Tests for outbound checkpoint persistence timing (Step 1.1).
//!
//! Verifies that `dispatch_and_persist` persists the checkpoint *before*
//! calling `plugin.send()` (mark_sent=false) and updates it *after*
//! successful delivery (mark_sent=true).

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

/// Verify that `dispatch_and_persist` persists the checkpoint with
/// mark_sent=false *before* calling plugin.send(), and with
/// mark_sent=true *after* successful delivery.
#[tokio::test]
async fn test_checkpoint_persisted_before_send_then_marked_sent() {
    let persist = Arc::new(TimingMockPersist::new());
    let setup = setup_timing_gw(Arc::clone(&persist)).await;

    let gw_arc = Arc::new(setup.gw);
    let sid = setup.session_id.clone();
    let gw_clone = Arc::clone(&gw_arc);
    let handle = tokio::spawn(async move {
        gw_clone
            .send_outbound(&sid, "mock", "hello world", vec![])
            .await
    });

    // Wait for send() to be entered (first persist has happened).
    setup.entered_send.notified().await;

    // At this point, the first persist (mark_sent=false) has completed
    // and plugin.send() is blocked. Verify the intermediate state.
    let saves = persist.get_saves().await;
    assert_eq!(saves.len(), 1, "should have 1 save before send completes");
    assert_eq!(saves[0].session_id, setup.session_id);
    assert_eq!(saves[0].pending_count, 1);
    assert!(
        !saves[0].last_pending_sent,
        "first persist should NOT be marked sent"
    );

    // Let send() complete. The task will continue and do the second persist.
    setup.ok_to_return.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(result.is_ok(), "send_outbound should succeed");

    // After send completes, verify the second persist (mark_sent=true).
    let saves = persist.get_saves().await;
    assert_eq!(saves.len(), 2, "should have 2 saves total");
    assert!(
        saves[1].last_pending_sent,
        "second persist should be marked sent"
    );
}

/// Verify that both text and interactive message types go through
/// the two-phase persist flow.
#[tokio::test]
async fn test_interactive_message_two_phase_persist() {
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
            .send_outbound(&sid, "mock", "hello interactive", vec![])
            .await
    });

    entered.notified().await;

    let saves = persist.get_saves().await;
    assert_eq!(saves.len(), 1, "should have 1 save before send");
    assert!(!saves[0].last_pending_sent, "should not be sent yet");

    ok.notify_one();
    let result = handle.await.expect("task should not panic");
    assert!(result.is_ok());

    let saves = persist.get_saves().await;
    assert_eq!(saves.len(), 2, "should have 2 saves");
    assert!(saves[1].last_pending_sent, "should be marked sent");
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
