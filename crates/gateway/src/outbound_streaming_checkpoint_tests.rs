//! Tests for streaming outbound checkpoint persistence (Step 1.2).
//!
//! Verifies that `send_outbound_streaming_inner` persists a checkpoint
//! after streaming completes, mirroring the batch path in
//! `dispatch_and_persist`.

use crate::{GatewayConfig, Message, SessionManager};
use closeclaw_common::im_plugin::{
    AdapterError, NormalizedMessage, RenderedOutput, StreamingOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult, StreamEvent};
use closeclaw_common::{IMPlugin, StreamingRenderer};
use closeclaw_llm::types::UnifiedUsage;
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Mock persistence ────────────────────────────────────────────────────────

/// Mock persistence that records saves and stores checkpoints.
struct StreamingCheckpointMockPersist {
    checkpoints: Mutex<HashMap<String, SessionCheckpoint>>,
    saves: Mutex<Vec<SaveRecord>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SaveRecord {
    session_id: String,
    pending_count: usize,
    last_pending_sent: bool,
}

impl StreamingCheckpointMockPersist {
    fn new() -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
            saves: Mutex::new(Vec::new()),
        }
    }

    async fn get_saves(&self) -> Vec<SaveRecord> {
        self.saves.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl PersistenceService for StreamingCheckpointMockPersist {
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

/// Mock persistence that always fails on save.
struct FailingSavePersist;

#[async_trait::async_trait]
impl PersistenceService for FailingSavePersist {
    async fn save_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "simulated save failure",
        )))
    }

    async fn load_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
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

// ── Mock plugin ─────────────────────────────────────────────────────────────

/// Capturing plugin that records all sent payloads.
struct CheckpointCapturingPlugin {
    platform: String,
    sent: Mutex<Vec<serde_json::Value>>,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl CheckpointCapturingPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            sent: Mutex::new(Vec::new()),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }

    async fn drain_sent(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut *self.sent.lock().await)
    }
}

#[async_trait::async_trait]
impl IMPlugin for CheckpointCapturingPlugin {
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
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.sent.lock().await.push(output.payload.clone());
        Ok(())
    }

    fn send_thinking_indicator(&self, _active: bool) {}

    fn handle_stream_event(&self, event: StreamEvent) -> StreamingOutput {
        self.renderer.lock().expect("lock").handle_event(event)
    }

    fn flush_stream(&self) -> StreamingOutput {
        self.renderer.lock().expect("lock").flush()
    }
}

// ── Setup helpers ───────────────────────────────────────────────────────────

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-streaming-checkpoint".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

/// Setup a Gateway with a checkpoint_manager backed by the given persistence.
async fn setup_with_checkpoint(
    persistence: Arc<dyn PersistenceService>,
    plugin: Arc<dyn IMPlugin>,
) -> (crate::Gateway, Arc<SessionManager>, String) {
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::clone(&persistence)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(plugin).await;
    let msg = Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: "agent-1".to_string(),
        content: "hello".to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    let cm = Arc::new(closeclaw_session::checkpoint_manager::CheckpointManager::new(persistence));
    let gw = gw.with_checkpoint_manager(cm);
    (gw, sm, sid)
}

/// Setup a Gateway WITHOUT a checkpoint_manager.
async fn setup_without_checkpoint(plugin: Arc<dyn IMPlugin>) -> (crate::Gateway, String) {
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(plugin).await;
    let msg = Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: "agent-1".to_string(),
        content: "hello".to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    (gw, sid)
}

fn default_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: Some(0),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

fn text_stream_events(text: &str) -> Vec<Result<StreamEvent, String>> {
    vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: text.to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ]
}

fn multi_block_stream_events() -> Vec<Result<StreamEvent, String>> {
    vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "Hello ".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: closeclaw_common::ContentDelta::Text {
                text: "world!".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ]
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 1: Normal path — checkpoint persisted correctly after streaming
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that after streaming completes, `persist_outbound_checkpoint`
/// is called with the correct content_blocks and mark_sent=true.
#[tokio::test]
async fn test_streaming_checkpoint_persisted_after_completion() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_checkpoint(
        Arc::clone(&persist) as Arc<dyn PersistenceService>,
        plugin.clone(),
    )
    .await;

    let events = text_stream_events("Hello streaming checkpoint!");
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok(), "streaming should succeed");

    // CheckpointManager::save() spawns async persistence; sleep to let it complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Verify checkpoint was persisted.
    let saves = persist.get_saves().await;
    // Filter saves to only those from our streaming call (mark_sent=true).
    let streaming_saves: Vec<_> = saves.iter().filter(|s| s.last_pending_sent).collect();
    assert_eq!(streaming_saves.len(), 1, "should have 1 streaming save");
    assert_eq!(streaming_saves[0].session_id, sid);

    // Verify checkpoint content_blocks contain the streamed text.
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
    assert_eq!(
        pending.content, "Hello streaming checkpoint!",
        "message content should match streamed text"
    );
    // content_blocks JSON should be present and contain the text.
    let cb: Vec<ContentBlock> = pending
        .content_blocks
        .as_ref()
        .map(|s| serde_json::from_str(s).unwrap())
        .unwrap_or_default();
    assert_eq!(cb.len(), 1, "should have 1 content block");
    assert!(
        matches!(&cb[0], ContentBlock::Text(t) if t == "Hello streaming checkpoint!"),
        "content block text should match"
    );
}

/// Verify checkpoint persistence with multiple content blocks.
#[tokio::test]
async fn test_streaming_checkpoint_multiple_blocks() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_checkpoint(
        Arc::clone(&persist) as Arc<dyn PersistenceService>,
        plugin.clone(),
    )
    .await;

    let events = multi_block_stream_events();
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok());

    // CheckpointManager::save() spawns async persistence; sleep to let it complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let streaming_saves: Vec<_> = persist
        .get_saves()
        .await
        .iter()
        .filter(|s| s.last_pending_sent)
        .cloned()
        .collect();
    assert_eq!(streaming_saves.len(), 1);

    // Verify checkpoint content_blocks contain both streamed blocks.
    let cp = persist
        .checkpoints
        .lock()
        .await
        .get(&sid)
        .cloned()
        .expect("checkpoint should exist");
    let pending = cp
        .outbound_pending
        .last()
        .expect("should have pending message");
    let cb: Vec<ContentBlock> = pending
        .content_blocks
        .as_ref()
        .map(|s| serde_json::from_str(s).unwrap())
        .unwrap_or_default();
    assert_eq!(cb.len(), 2, "should have 2 content blocks");
    assert!(
        matches!(&cb[0], ContentBlock::Text(t) if t == "Hello "),
        "first block should be 'Hello '"
    );
    assert!(
        matches!(&cb[1], ContentBlock::Text(t) if t == "world!"),
        "second block should be 'world!'"
    );
}

/// Verify that the platform field is set in the persisted checkpoint.
#[tokio::test]
async fn test_streaming_checkpoint_platform_field() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_checkpoint(
        Arc::clone(&persist) as Arc<dyn PersistenceService>,
        plugin.clone(),
    )
    .await;

    let events = text_stream_events("platform test");
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok());

    // CheckpointManager::save() spawns async persistence; sleep to let it complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let cp = persist
        .checkpoints
        .lock()
        .await
        .get(&sid)
        .cloned()
        .expect("checkpoint should exist");
    let pending = cp
        .outbound_pending
        .last()
        .expect("should have pending message");
    assert_eq!(
        pending.platform.as_deref(),
        Some("mock"),
        "platform should be set to channel name"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 2: No checkpoint_manager — silently skips persistence
// ═══════════════════════════════════════════════════════════════════════════

/// When no checkpoint_manager is configured, streaming should succeed
/// without panicking or persisting a checkpoint.
#[tokio::test]
async fn test_streaming_no_checkpoint_manager_no_panic() {
    let plugin = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, sid) = setup_without_checkpoint(plugin.clone()).await;

    let events = text_stream_events("no checkpoint manager");
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(
        result.is_ok(),
        "streaming should succeed without checkpoint_manager"
    );

    // Verify the text was still sent via the plugin.
    let sent = plugin.drain_sent().await;
    assert_eq!(sent.len(), 1, "plugin.send should still be called");
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 3: Checkpoint persistence failure — does not panic, only warns
// ═══════════════════════════════════════════════════════════════════════════

/// When save_checkpoint fails, streaming should still succeed without
/// panicking. The checkpoint error is logged as a warning, not propagated.
#[tokio::test]
async fn test_streaming_checkpoint_save_failure_no_panic() {
    let persist: Arc<dyn PersistenceService> = Arc::new(FailingSavePersist);
    let plugin = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_checkpoint(persist, plugin.clone()).await;

    let events = text_stream_events("save failure test");
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(
        result.is_ok(),
        "streaming should succeed even when checkpoint save fails"
    );

    // Verify the text was still sent via the plugin.
    let sent = plugin.drain_sent().await;
    assert_eq!(sent.len(), 1, "plugin.send should still be called");
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 4: Streaming checkpoint includes dsl_result (Step 1.3)
// ═══════════════════════════════════════════════════════════════════════════

/// Setup a Gateway with a ProcessorRegistry containing DslParser,
/// so the outbound chain produces dsl_result in metadata.
async fn setup_with_dsl_parser(
    persistence: Arc<dyn PersistenceService>,
    plugin: Arc<dyn IMPlugin>,
) -> (crate::Gateway, Arc<SessionManager>, String) {
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::clone(&persistence)),
        None,
        ReasoningLevel::default(),
    ));
    // Build a minimal outbound chain with only DslParser.
    let mut registry = closeclaw_processor_chain::registry::ProcessorRegistry::new();
    registry.register(Arc::new(closeclaw_processor_chain::DslParser));
    let gw = crate::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_plugin(plugin).await;
    let msg = Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: "agent-1".to_string(),
        content: "hello".to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    let cm = Arc::new(closeclaw_session::checkpoint_manager::CheckpointManager::new(
        persistence,
    ));
    let gw = gw.with_checkpoint_manager(cm);
    (gw, sm, sid)
}

/// Stream events containing a DSL instruction (::button[...]).
fn dsl_stream_events() -> Vec<Result<StreamEvent, String>> {
    vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "Please confirm:\n::button[label:Yes;action:confirm;value:1]".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ]
}

/// Verify that when streaming content contains DSL instructions,
/// the persisted checkpoint includes a non-empty dsl_result.
#[tokio::test]
async fn test_streaming_checkpoint_dsl_result_present() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_dsl_parser(
        Arc::clone(&persist) as Arc<dyn PersistenceService>,
        plugin.clone(),
    )
    .await;

    let events = dsl_stream_events();
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok(), "streaming should succeed");

    // The StreamResult itself should carry dsl_result.
    let sr = result.unwrap();
    assert!(
        sr.dsl_result.is_some(),
        "StreamResult.dsl_result should be Some when DSL is present"
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Verify checkpoint was persisted with dsl_result.
    let streaming_saves: Vec<_> = persist
        .get_saves()
        .await
        .iter()
        .filter(|s| s.last_pending_sent)
        .cloned()
        .collect();
    assert_eq!(streaming_saves.len(), 1, "should have 1 streaming save");

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
    assert!(
        pending.dsl_result.is_some(),
        "checkpoint pending message dsl_result should be Some"
    );
    // The dsl_result JSON should contain at least one instruction.
    let dsl: DslParseResult =
        serde_json::from_str(pending.dsl_result.as_ref().unwrap()).unwrap();
    assert_eq!(dsl.instructions.len(), 1, "should have 1 DSL instruction");
    assert_eq!(dsl.instructions[0].instruction_type, "button");
    assert_eq!(dsl.instructions[0].params["label"], "Yes");
}

/// Verify that streaming without DSL instructions produces None dsl_result
/// in the checkpoint when no processor chain is configured, consistent
/// with the batch path behavior.
#[tokio::test]
async fn test_streaming_checkpoint_dsl_result_absent_when_no_dsl() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(CheckpointCapturingPlugin::new("mock"));
    // Build a Gateway with an empty processor registry (no DslParser).
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    let empty_registry = closeclaw_processor_chain::registry::ProcessorRegistry::new();
    let gw = crate::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(empty_registry),
    );
    gw.register_plugin(plugin.clone()).await;
    let msg = Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: "agent-1".to_string(),
        content: "hello".to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    let cm = Arc::new(closeclaw_session::checkpoint_manager::CheckpointManager::new(
        persist.clone() as Arc<dyn PersistenceService>,
    ));
    let gw = gw.with_checkpoint_manager(cm);

    // Stream plain text — no DSL instructions and no DslParser.
    let events = text_stream_events("Hello, no DSL here!");
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok());

    let sr = result.unwrap();
    assert!(
        sr.dsl_result.is_none(),
        "StreamResult.dsl_result should be None when no processor chain runs"
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
    assert!(
        pending.dsl_result.is_none(),
        "checkpoint pending message dsl_result should be None when no processor chain"
    );
}

/// Verify that when DslParser runs but finds no DSL instructions,
/// the dsl_result is Some(empty) — not None — because DslParser
/// always inserts its result into metadata.
#[tokio::test]
async fn test_streaming_checkpoint_dsl_result_empty_when_parser_finds_no_dsl() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_dsl_parser(
        Arc::clone(&persist) as Arc<dyn PersistenceService>,
        plugin.clone(),
    )
    .await;

    // Stream plain text — no DSL instructions, but DslParser is registered.
    let events = text_stream_events("Hello, no DSL here!");
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok());

    let sr = result.unwrap();
    // DslParser always inserts dsl_result, so it is Some with empty instructions.
    assert!(
        sr.dsl_result.is_some(),
        "StreamResult.dsl_result should be Some when DslParser runs (even with no DSL)"
    );
    let dsl: DslParseResult =
        serde_json::from_str(sr.dsl_result.as_ref().unwrap()).unwrap();
    assert!(
        dsl.instructions.is_empty(),
        "dsl_result instructions should be empty when no DSL in content"
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
    assert!(pending.dsl_result.is_some());
    let dsl_cp: DslParseResult =
        serde_json::from_str(pending.dsl_result.as_ref().unwrap()).unwrap();
    assert!(
        dsl_cp.instructions.is_empty(),
        "checkpoint dsl_result instructions should be empty"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 5: Empty content_blocks — checkpoint still persisted
// ═══════════════════════════════════════════════════════════════════════════

/// When streaming completes with no text content (empty content_blocks
/// after verbosity filtering), the checkpoint should still be persisted
/// with an empty message — consistent with the batch path.
#[tokio::test]
async fn test_streaming_empty_content_blocks_checkpoint_persisted() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_checkpoint(
        Arc::clone(&persist) as Arc<dyn PersistenceService>,
        plugin.clone(),
    )
    .await;

    // Stream with only newlines — after trimming/filtering, content_blocks may be empty.
    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "\n\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok());

    // CheckpointManager::save() spawns async persistence; sleep to let it complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Checkpoint should still be persisted even with empty/whitespace content.
    let streaming_saves: Vec<_> = persist
        .get_saves()
        .await
        .iter()
        .filter(|s| s.last_pending_sent)
        .cloned()
        .collect();
    assert_eq!(
        streaming_saves.len(),
        1,
        "checkpoint should be persisted even with empty content"
    );
}

/// Verify that an empty stream (no blocks at all) still persists a checkpoint.
#[tokio::test]
async fn test_streaming_no_blocks_checkpoint_persisted() {
    let persist = Arc::new(StreamingCheckpointMockPersist::new());
    let plugin = Arc::new(CheckpointCapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_with_checkpoint(
        Arc::clone(&persist) as Arc<dyn PersistenceService>,
        plugin.clone(),
    )
    .await;

    // Stream with only MessageEnd — no content blocks at all.
    let events = vec![Ok::<_, String>(StreamEvent::MessageEnd {
        usage: Some(default_usage()),
        finish_reason: Some("stop".to_string()),
    })];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;
    assert!(result.is_ok());

    // CheckpointManager::save() spawns async persistence; sleep to let it complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Checkpoint should still be persisted.
    let streaming_saves: Vec<_> = persist
        .get_saves()
        .await
        .iter()
        .filter(|s| s.last_pending_sent)
        .cloned()
        .collect();
    assert_eq!(streaming_saves.len(), 1, "checkpoint should be persisted");

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
    assert_eq!(pending.content, "", "message content should be empty");
}
