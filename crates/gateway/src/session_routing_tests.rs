//! Unit tests for session routing failure in `handle_inbound_message`.
//!
//! Covers the behavior changed in Step 1.1: when `resolve_session_from_message`
//! returns `None` (session_key empty or SessionManager::resolve fails), the
//! gateway replies with "会话路由失败，请重试" via the simplified outbound path
//! (render → send), consistent with non-text message interception.

use crate::compute_session_key;
use crate::{GatewayConfig, HandleResult, Message, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin};
use closeclaw_common::processor::{DslParseResult, ProcessError, ProcessedMessage, ProcessorChain};
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

// ── Mock plugin ─────────────────────────────────────────────────────────────

/// Captures `render` and `send` invocations so tests can assert on
/// the outbound flow used by `send_outbound_simplified`.
struct CapturingPlugin {
    platform: String,
    render_calls: std::sync::Mutex<Vec<Vec<ContentBlock>>>,
    send_calls: std::sync::Mutex<Vec<(RenderedOutput, String, Option<String>)>>,
}

impl CapturingPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            render_calls: std::sync::Mutex::new(Vec::new()),
            send_calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn render_count(&self) -> usize {
        self.render_calls.lock().unwrap().len()
    }

    fn send_count(&self) -> usize {
        self.send_calls.lock().unwrap().len()
    }

    fn last_send(&self) -> Option<(RenderedOutput, String, Option<String>)> {
        self.send_calls.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl IMPlugin for CapturingPlugin {
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
        self.render_calls
            .lock()
            .unwrap()
            .push(content_blocks.to_vec());
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
            payload: json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.send_calls.lock().unwrap().push((
            RenderedOutput {
                msg_type: output.msg_type.clone(),
                payload: output.payload.clone(),
            },
            peer_id.to_string(),
            thread_id.map(|s| s.to_string()),
        ));
        Ok(())
    }
}

// ── Mock processor chain ────────────────────────────────────────────────────

/// Records `process_outbound` invocations so tests can verify the outbound
/// processor chain is NOT exercised during `send_outbound_simplified`.
struct RecordingProcessorChain {
    outbound_calls: std::sync::Mutex<usize>,
}

impl RecordingProcessorChain {
    fn new() -> Self {
        Self {
            outbound_calls: std::sync::Mutex::new(0),
        }
    }

    fn outbound_call_count(&self) -> usize {
        *self.outbound_calls.lock().unwrap()
    }
}

#[async_trait]
impl ProcessorChain for RecordingProcessorChain {
    async fn process_inbound(
        &self,
        msg: NormalizedMessage,
    ) -> Result<ProcessedMessage, ProcessError> {
        Ok(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(msg.content)],
            metadata: HashMap::new(),
        })
    }

    async fn process_outbound(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, ProcessError> {
        *self.outbound_calls.lock().unwrap() += 1;
        Ok(msg)
    }

    async fn process_outbound_raw_log_only(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, ProcessError> {
        // No-op: the simplified path should not invoke the full chain.
        Ok(msg)
    }

    fn inbound_len(&self) -> usize {
        0
    }

    fn outbound_len(&self) -> usize {
        0
    }
}

// ── Test helpers ────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_message(to: &str, content: &str) -> Message {
    Message {
        id: "msg_1".to_string(),
        from: "ou_sender".to_string(),
        to: to.to_string(),
        content: content.to_string(),
        channel: "mock".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
    }
}

async fn make_gw(channel: &str) -> (crate::Gateway, Arc<CapturingPlugin>) {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let capturing: Arc<CapturingPlugin> = Arc::new(CapturingPlugin::new(channel));
    let plugin: Arc<dyn IMPlugin> = capturing.clone() as Arc<dyn IMPlugin>;
    gw.register_plugin(plugin).await;
    (gw, capturing)
}

/// Build a Gateway with a recording outbound processor chain.
async fn make_gw_with_processor(
    channel: &str,
) -> (
    crate::Gateway,
    Arc<CapturingPlugin>,
    Arc<RecordingProcessorChain>,
) {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let chain: Arc<RecordingProcessorChain> = Arc::new(RecordingProcessorChain::new());
    let gw = crate::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        chain.clone() as Arc<dyn ProcessorChain>,
    );
    let capturing: Arc<CapturingPlugin> = Arc::new(CapturingPlugin::new(channel));
    let plugin: Arc<dyn IMPlugin> = capturing.clone() as Arc<dyn IMPlugin>;
    gw.register_plugin(plugin).await;
    (gw, capturing, chain)
}

/// Build a Gateway whose SessionManager will fail on resolve because
/// `workspace_dir` points to an inaccessible location.
async fn make_gw_with_failing_resolve(channel: &str) -> (crate::Gateway, Arc<CapturingPlugin>) {
    let config = make_config();
    // Use /proc as workspace_dir — create_dir_all will fail on Linux
    // because /proc is a read-only virtual filesystem.
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        Some(PathBuf::from("/proc")),
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let capturing: Arc<CapturingPlugin> = Arc::new(CapturingPlugin::new(channel));
    let plugin: Arc<dyn IMPlugin> = capturing.clone() as Arc<dyn IMPlugin>;
    gw.register_plugin(plugin).await;
    (gw, capturing)
}

/// Build a `ProcessedMessage` with empty session_key (routing failure).
fn make_processed_no_session_key(msg: &Message, _channel: &str) -> ProcessedMessage {
    let mut metadata = HashMap::new();
    // Intentionally omit session_key or set it to empty.
    metadata.insert("session_key".to_string(), String::new());
    metadata.insert("peer_id".to_string(), msg.to.clone());
    metadata.insert("sender_id".to_string(), msg.from.clone());
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata,
    }
}

/// Build a `ProcessedMessage` with a non-empty session_key (for resolve
/// failure test).
fn make_processed_with_session_key(msg: &Message, channel: &str) -> ProcessedMessage {
    let session_key = compute_session_key(channel, &msg.from, &msg.to, None, msg.timestamp);
    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), session_key);
    metadata.insert("peer_id".to_string(), msg.to.clone());
    metadata.insert("sender_id".to_string(), msg.from.clone());
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata,
    }
}

/// Build a `ProcessedMessage` with empty peer_id.
fn make_processed_empty_peer_id(msg: &Message, _channel: &str) -> ProcessedMessage {
    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), String::new());
    metadata.insert("peer_id".to_string(), String::new()); // empty
    metadata.insert("sender_id".to_string(), msg.from.clone());
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata,
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Normal path — session_key empty triggers routing failure reply
// ═════════════════════════════════════════════════════════════════════════════

/// When session_key is empty, handle_inbound_message returns None and
/// sends the error reply via the simplified outbound path (render + send).
#[tokio::test]
async fn test_session_key_empty_returns_none_with_reply() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");

    let processed = make_processed_no_session_key(&msg, "mock");
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "session_key empty should return None");
    assert_eq!(plugin.render_count(), 1, "render should be called once");
    assert_eq!(plugin.send_count(), 1, "send should be called once");
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Message content — error reply text matches design doc
// ═════════════════════════════════════════════════════════════════════════════

/// The error reply sent when session_key is empty must contain the exact
/// text "会话路由失败，请重试" per the design doc.
#[tokio::test]
async fn test_session_routing_failure_message_text() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");

    let processed = make_processed_no_session_key(&msg, "mock");
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none());
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");

    let (output, peer_id, _thread_id) = plugin.last_send().unwrap();
    assert_eq!(output.msg_type, "text");
    assert_eq!(peer_id, "agent-1");
    let text = output.payload["content"]["text"].as_str().unwrap();
    assert_eq!(
        text, "会话路由失败，请重试",
        "error text must match design doc: got {text}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. peer_id empty — should not send, return None silently
// ═════════════════════════════════════════════════════════════════════════════

/// When peer_id metadata is empty, the routing failure path should return
/// None without calling plugin.send() (no target to send to).
#[tokio::test]
async fn test_session_routing_failure_empty_peer_id_skips_send() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");

    let processed = make_processed_empty_peer_id(&msg, "mock");
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(
        result.is_none(),
        "empty peer_id with empty session_key should return None"
    );
    assert_eq!(
        plugin.send_count(),
        0,
        "no send should occur when peer_id is empty"
    );
    assert_eq!(
        plugin.render_count(),
        0,
        "no render should occur when peer_id is empty"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. SessionManager::resolve fails — same simplified outbound path
// ═════════════════════════════════════════════════════════════════════════════

/// When session_key is non-empty but SessionManager::resolve fails (e.g.
/// workspace creation error), the routing failure reply is sent through
/// the same simplified outbound path.
#[tokio::test]
async fn test_resolve_failure_uses_simplified_outbound() {
    let (gw, plugin) = make_gw_with_failing_resolve("mock").await;
    let msg = make_message("agent-1", "hello");

    let processed = make_processed_with_session_key(&msg, "mock");
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "resolve failure should return None");
    // Simplified outbound path: render + send each called once.
    assert_eq!(plugin.render_count(), 1, "render should be called once");
    assert_eq!(plugin.send_count(), 1, "send should be called once");

    // Verify the error message content.
    let (output, peer_id, _thread_id) = plugin.last_send().unwrap();
    assert_eq!(output.msg_type, "text");
    assert_eq!(peer_id, "agent-1");
    let text = output.payload["content"]["text"].as_str().unwrap();
    assert_eq!(
        text, "会话路由失败，请重试",
        "resolve failure reply must match design doc: got {text}"
    );
}

/// When resolve fails and peer_id is empty, no send should occur.
#[tokio::test]
async fn test_resolve_failure_empty_peer_id_skips_send() {
    let (gw, plugin) = make_gw_with_failing_resolve("mock").await;
    let msg = make_message("agent-1", "hello");

    // Build processed message with non-empty session_key but empty peer_id.
    let session_key = compute_session_key("mock", &msg.from, &msg.to, None, msg.timestamp);
    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), session_key);
    metadata.insert("peer_id".to_string(), String::new()); // empty
    metadata.insert("sender_id".to_string(), msg.from.clone());
    let processed = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata,
    };

    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(
        result.is_none(),
        "resolve failure with empty peer_id should return None"
    );
    assert_eq!(
        plugin.send_count(),
        0,
        "no send when peer_id is empty even on resolve failure"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Outbound processor chain is bypassed for routing failure replies
// ═════════════════════════════════════════════════════════════════════════════

/// The routing failure reply must go through the simplified outbound path,
/// NOT the full outbound processor chain (no Verbosity/DslParser).
#[tokio::test]
async fn test_routing_failure_skips_outbound_processor_chain() {
    let (gw, plugin, chain) = make_gw_with_processor("mock").await;
    let msg = make_message("agent-1", "hello");

    let processed = make_processed_no_session_key(&msg, "mock");
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "session_key empty should return None");
    assert_eq!(
        chain.outbound_call_count(),
        0,
        "outbound processor chain should be bypassed"
    );
    assert_eq!(plugin.send_count(), 1, "error reply should still be sent");
}
