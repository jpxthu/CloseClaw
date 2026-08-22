//! Step 1.5c tests: state transition, finish VerbosityFilter skip, edge
//! cases, and batch success regression.
//!
//! Covers the remaining test dimensions not addressed by Step 1.5a (registry +
//! incremental DSL) and Step 1.5b (batch failure + middleware rejection):
//!
//! - **State transition**: incremental DSL instructions accumulate in
//!   `StreamState::dsl_instructions` and merge into the final `dsl_result`.
//! - **Finish phase skips VerbosityFilter**: `finish_streaming_pipeline` calls
//!   `process_outbound_without_verbosity`, which runs DslParser but skips
//!   VerbosityFilter.
//! - **Edge cases**: empty chunk, empty line DSL (no instructions), registry
//!   `None` passthrough.
//! - **Batch success**: no failure notification when `plugin.send` succeeds.
//! - **Regression**: pre-flight rejection and stream error paths unaffected.

mod part2;

use crate::{Gateway, GatewayConfig, Message, OutboundMeta, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, DslInstruction, DslParseResult, ProcessedMessage};
use closeclaw_common::StreamingRenderer;
use closeclaw_llm::types::{ContentBlockType, ContentDelta, StreamEvent, UnifiedUsage};
use closeclaw_session::persistence::{PersistenceError, ReasoningLevel, SessionCheckpoint};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

// ── Mock ProcessorChain (incremental DSL parsing) ──────────────────────────

pub struct MockChain {
    parsed: StdMutex<Vec<String>>,
    instructions: StdMutex<Vec<DslInstruction>>,
}

impl MockChain {
    pub fn new() -> Self {
        Self {
            parsed: StdMutex::new(Vec::new()),
            instructions: StdMutex::new(Vec::new()),
        }
    }

    pub fn push_instruction(&self, i: DslInstruction) {
        self.instructions.lock().unwrap().push(i);
    }
}

#[async_trait]
impl closeclaw_common::processor::ProcessorChain for MockChain {
    async fn process_inbound(
        &self,
        msg: closeclaw_common::im_plugin::NormalizedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        Ok(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(msg.content)],
            metadata: HashMap::new(),
        })
    }

    async fn process_outbound(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        Ok(msg)
    }

    async fn process_outbound_incremental(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        // Simulate the real ProcessorRegistry: incremental phase skips
        // DslParser (zero-overhead passthrough) and OutboundRawLog.
        // Only VerbosityFilter executes here. Text is preserved as-is.
        let metadata = msg.metadata;
        // Still record DSL lines for test observability.
        for block in &msg.content_blocks {
            if let ContentBlock::Text(t) = block {
                let trimmed = t.trim();
                if trimmed.starts_with("::button[") || trimmed.starts_with("::selector[") {
                    self.parsed.lock().unwrap().push(t.clone());
                }
            }
        }
        Ok(ProcessedMessage {
            content_blocks: msg.content_blocks,
            metadata,
        })
    }

    fn parse_line_for_dsl(&self, line: &str) -> (String, DslParseResult) {
        self.parsed.lock().unwrap().push(line.to_string());
        let trimmed = line.trim();
        if trimmed.starts_with("::button[") || trimmed.starts_with("::selector[") {
            let mut q = self.instructions.lock().unwrap();
            if !q.is_empty() {
                let instr = q.remove(0);
                return (
                    String::new(),
                    DslParseResult {
                        instructions: vec![instr],
                    },
                );
            }
        }
        (
            line.to_string(),
            DslParseResult {
                instructions: vec![],
            },
        )
    }

    fn inbound_len(&self) -> usize {
        0
    }

    fn outbound_len(&self) -> usize {
        0
    }
}

// ── Shared mock helpers ────────────────────────────────────────────────────

/// Shared render logic: joins Text blocks into a single string.
fn mock_render(
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

/// Controls whether `send` succeeds or always fails.
pub(crate) enum SendBehavior {
    Ok,
    Fail,
}

/// Unified mock plugin: captures sent payloads and optionally fails on `send`.
/// Used by CapturingPlugin (success) and FailingPlugin (failure) via `new()`
/// constructors below.
pub(crate) struct MockImPlugin {
    platform: String,
    sent: StdMutex<Vec<serde_json::Value>>,
    call_count: AtomicU32,
    send_behavior: SendBehavior,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl MockImPlugin {
    fn new(platform: &str, send_behavior: SendBehavior) -> Self {
        Self {
            platform: platform.to_string(),
            sent: StdMutex::new(Vec::new()),
            call_count: AtomicU32::new(0),
            send_behavior,
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }

    fn drain_sent(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }

    fn send_count(&self) -> u32 {
        self.call_count.load(Ordering::SeqCst)
    }

    fn streaming_renderer(
        &self,
    ) -> Option<&std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>> {
        Some(&self.renderer)
    }
}

#[async_trait]
impl IMPlugin for MockImPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        mock_render(content_blocks, dsl_result)
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.sent.lock().unwrap().push(output.payload.clone());
        match self.send_behavior {
            SendBehavior::Ok => Ok(()),
            SendBehavior::Fail => Err(AdapterError::SendFailed("network error".into())),
        }
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .expect("MockImPlugin has no streaming renderer")
            .lock()
            .expect("MockImPlugin streaming renderer lock poisoned")
            .handle_event(event)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .expect("MockImPlugin has no streaming renderer")
            .lock()
            .expect("MockImPlugin streaming renderer lock poisoned")
            .flush()
    }
}

// ── CapturingPlugin / FailingPlugin type aliases ────────────────────────────

/// Alias for `MockImPlugin` that always succeeds on send.
pub struct CapturingPlugin(MockImPlugin);

impl CapturingPlugin {
    pub fn new(platform: &str) -> Self {
        Self(MockImPlugin::new(platform, SendBehavior::Ok))
    }
    pub fn drain_sent(&self) -> Vec<serde_json::Value> {
        self.0.drain_sent()
    }
    pub fn send_count(&self) -> u32 {
        self.0.send_count()
    }
}

#[async_trait]
impl IMPlugin for CapturingPlugin {
    fn platform(&self) -> &str {
        self.0.platform()
    }
    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
        self.0.parse_inbound(payload).await
    }
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        self.0.render(content_blocks, dsl_result)
    }
    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.0.send(output, peer_id, thread_id).await
    }
    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.0.handle_stream_event(event)
    }
    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.0.flush_stream()
    }
}

/// Alias for `MockImPlugin` that always fails on send.
pub struct FailingPlugin(MockImPlugin);

#[async_trait]
impl IMPlugin for FailingPlugin {
    fn platform(&self) -> &str {
        self.0.platform()
    }
    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
        self.0.parse_inbound(payload).await
    }
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        self.0.render(content_blocks, dsl_result)
    }
    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.0.send(output, peer_id, thread_id).await
    }
    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.0.handle_stream_event(event)
    }
    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.0.flush_stream()
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

pub fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-15c".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

pub fn make_message(to: &str, content: &str) -> Message {
    Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: to.to_string(),
        content: content.to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

pub struct MockPersist;

#[async_trait]
impl closeclaw_session::persistence::PersistenceService for MockPersist {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(Some(SessionCheckpoint::new("mock".into())))
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn purge_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

pub async fn setup(
    chain: Arc<dyn closeclaw_common::processor::ProcessorChain>,
    plugin: Arc<dyn IMPlugin>,
) -> (Gateway, Arc<SessionManager>, String) {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersist)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), chain);
    gw.register_plugin(plugin).await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    (gw, sm, sid)
}

pub fn default_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: Some(0),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

pub fn extract_text(payload: &serde_json::Value) -> String {
    payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}
