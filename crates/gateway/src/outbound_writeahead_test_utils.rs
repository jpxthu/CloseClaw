//! Shared test utilities for outbound write-ahead integration tests.

use crate::{GatewayConfig, SessionManager};
use closeclaw_common::im_plugin::{
    AdapterError, NormalizedMessage, RenderedOutput, StreamingOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult, StreamEvent};
use closeclaw_common::IMPlugin;
use closeclaw_session::persistence::{
    PendingOperation, PendingOperationStatus, PendingOperationType, PersistenceError,
    PersistenceService, SessionCheckpoint,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, Notify};

// ---------------------------------------------------------------------------
// Mock persistence — records saves with op snapshots
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct SaveSnapshot {
    pub(crate) outbound_ops_count: usize,
    pub(crate) outbound_pending_count: usize,
}

pub(crate) struct SnapshotMockPersist {
    pub(crate) checkpoints: Mutex<HashMap<String, SessionCheckpoint>>,
    pub(crate) snapshots: Arc<StdMutex<Vec<SaveSnapshot>>>,
    /// Fail on the Nth save call (0-indexed).
    fail_on_save: StdMutex<Option<usize>>,
    call_count: StdMutex<usize>,
}

impl SnapshotMockPersist {
    pub(crate) fn new() -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
            snapshots: Arc::new(StdMutex::new(Vec::new())),
            fail_on_save: StdMutex::new(None),
            call_count: StdMutex::new(0),
        }
    }

    pub(crate) fn fail_on_save(n: usize) -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
            snapshots: Arc::new(StdMutex::new(Vec::new())),
            fail_on_save: StdMutex::new(Some(n)),
            call_count: StdMutex::new(0),
        }
    }

    pub(crate) fn snapshots(&self) -> Vec<SaveSnapshot> {
        self.snapshots.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl PersistenceService for SnapshotMockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        let n = {
            let mut count = self.call_count.lock().unwrap();
            let n = *count;
            *count += 1;
            n
        };
        if let Some(fail_at) = *self.fail_on_save.lock().unwrap() {
            if n == fail_at {
                return Err(PersistenceError::Lock(format!(
                    "simulated failure on save #{}",
                    n
                )));
            }
        }
        let outbound_ops = cp
            .pending_operations
            .iter()
            .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
            .count();
        self.snapshots.lock().unwrap().push(SaveSnapshot {
            outbound_ops_count: outbound_ops,
            outbound_pending_count: cp.outbound_pending.len(),
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
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Mock plugin — synchronized two-phase send
// ---------------------------------------------------------------------------

pub(crate) struct SyncPlugin {
    entered_send: Arc<Notify>,
    ok_to_return: Arc<Notify>,
    pub(crate) sent_texts: Arc<Mutex<Vec<String>>>,
}

impl SyncPlugin {
    pub(crate) fn new() -> (Self, Arc<Notify>, Arc<Notify>, Arc<Mutex<Vec<String>>>) {
        let entered = Arc::new(Notify::new());
        let ok = Arc::new(Notify::new());
        let texts = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                entered_send: Arc::clone(&entered),
                ok_to_return: Arc::clone(&ok),
                sent_texts: Arc::clone(&texts),
            },
            entered,
            ok,
            texts,
        )
    }
}

#[async_trait::async_trait]
impl IMPlugin for SyncPlugin {
    fn platform(&self) -> &str {
        "mock"
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
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output.payload["content"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.sent_texts.lock().await.push(text);
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
// Simple plugin that tracks sent texts
// ---------------------------------------------------------------------------

pub(crate) struct SimpleTrackPlugin {
    pub(crate) texts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl IMPlugin for SimpleTrackPlugin {
    fn platform(&self) -> &str {
        "mock"
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
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output.payload["content"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.texts.lock().await.push(text);
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
// Setup helpers
// ---------------------------------------------------------------------------

pub(crate) fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "step15-integration".into(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

/// Register a session in the SessionManager for testing.
pub(crate) async fn register_session(mgr: &SessionManager, session_id: &str, channel: &str) {
    mgr.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: "test-agent".into(),
            channel: channel.to_string(),
            created_at: 0,
            depth: 0,
        },
    );
}

pub(crate) async fn setup_gw_with_persist(
    persist: Arc<SnapshotMockPersist>,
    session_id: &str,
) -> (
    crate::Gateway,
    Arc<Notify>,
    Arc<Notify>,
    Arc<Mutex<Vec<String>>>,
) {
    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        closeclaw_session::persistence::ReasoningLevel::default(),
    ));
    register_session(&sm, session_id, "mock").await;
    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    let gw = crate::Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);

    let (plugin, entered, ok, texts) = SyncPlugin::new();
    gw.register_plugin(Arc::new(plugin) as Arc<dyn IMPlugin>)
        .await;

    (gw, entered, ok, texts)
}

pub(crate) fn make_outbound_op(message_id: &str, target_channel: &str) -> PendingOperation {
    PendingOperation {
        op_id: message_id.into(),
        op_type: PendingOperationType::OutboundMessage,
        status: PendingOperationStatus::Running,
        detail:
            closeclaw_session::pending_operation_detail::PendingOperationDetail::OutboundMessage {
                target_channel: target_channel.into(),
                message_id: message_id.into(),
                delivery_status: "pending".into(),
            },
        created_at: chrono::Utc::now(),
    }
}

pub(crate) fn make_tool_op(tool_id: &str, tool_name: &str) -> PendingOperation {
    PendingOperation {
        op_id: tool_id.into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: closeclaw_session::pending_operation_detail::PendingOperationDetail::ToolCall {
            tool_name: tool_name.into(),
            args_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    }
}

pub(crate) fn make_child_op(child_id: &str, agent_id: &str) -> PendingOperation {
    PendingOperation {
        op_id: child_id.into(),
        op_type: PendingOperationType::SubSessionSpawn,
        status: PendingOperationStatus::Running,
        detail:
            closeclaw_session::pending_operation_detail::PendingOperationDetail::SubSessionSpawn {
                child_session_id: child_id.into(),
                agent_id: agent_id.into(),
                task_summary: String::new(),
            },
        created_at: chrono::Utc::now(),
    }
}

pub(crate) fn outbound_ops_count(cp: &SessionCheckpoint) -> usize {
    cp.pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .count()
}
