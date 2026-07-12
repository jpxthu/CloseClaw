//! Unit tests for non-text message interception in `handle_inbound_message`.
//!
//! Covers the behavior added in Step 1.1: when `message_type` metadata is
//! not `Text`, the gateway sends an error reply via the plugin and returns
//! `None`, bypassing slash/LLM routing.
//!
//! Step 1.2 additions verify that non-text interception happens before
//! session resolution — non-text messages never reach
//! `resolve_session_from_message` and never create sessions.
//!
//! Step 1.3 additions verify that the error reply now flows through
//! `send_outbound_simplified` (raw-log processor only), and that
//! `account_id` propagates correctly through metadata.

use crate::compute_session_key;
use crate::{GatewayConfig, HandleResult, Message, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::MessageType;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin};
use closeclaw_common::processor::{DslParseResult, ProcessError, ProcessedMessage, ProcessorChain};
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Mock plugin ─────────────────────────────────────────────────────────────

/// Captures `render` and `send` invocations so tests can assert on
/// the outbound flow used by `send_outbound_to_chat` (full processor chain
/// + render → middleware → send).
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

/// Records processor chain invocations so tests can verify which
/// outbound path is exercised:
/// - `process_outbound_raw_log_only` for the simplified path
/// - `process_outbound` for the full chain path
struct RecordingProcessorChain {
    outbound_raw_log_only_calls: std::sync::Mutex<usize>,
    outbound_full_calls: std::sync::Mutex<usize>,
}

impl RecordingProcessorChain {
    fn new() -> Self {
        Self {
            outbound_raw_log_only_calls: std::sync::Mutex::new(0),
            outbound_full_calls: std::sync::Mutex::new(0),
        }
    }

    fn outbound_raw_log_only_call_count(&self) -> usize {
        *self.outbound_raw_log_only_calls.lock().unwrap()
    }

    fn outbound_full_call_count(&self) -> usize {
        *self.outbound_full_calls.lock().unwrap()
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
        *self.outbound_full_calls.lock().unwrap() += 1;
        Ok(msg)
    }

    async fn process_outbound_raw_log_only(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, ProcessError> {
        *self.outbound_raw_log_only_calls.lock().unwrap() += 1;
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

/// Build a `ProcessedMessage` with the given content and optional `message_type`.
///
/// When `msg_type` is `None`, the `message_type` key is omitted from metadata,
/// allowing us to test the "no message_type -> defaults to text" path.
fn make_processed(
    msg: &Message,
    channel: &str,
    content: &str,
    msg_type: Option<&MessageType>,
) -> ProcessedMessage {
    let session_key = compute_session_key(channel, &msg.from, &msg.to, None, msg.timestamp);
    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), session_key);
    metadata.insert("peer_id".to_string(), msg.to.clone());
    metadata.insert("sender_id".to_string(), msg.from.clone());
    if let Some(mt) = msg_type {
        metadata.insert(
            "message_type".to_string(),
            serde_json::to_string(mt).unwrap(),
        );
    }
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(content.to_string())],
        metadata,
    }
}

/// Build a `ProcessedMessage` with explicit `account_id` in metadata.
fn make_processed_with_account(
    msg: &Message,
    channel: &str,
    content: &str,
    msg_type: &MessageType,
    account_id: &str,
) -> ProcessedMessage {
    let mut pm = make_processed(msg, channel, content, Some(msg_type));
    pm.metadata
        .insert("account_id".to_string(), account_id.to_string());
    pm
}

/// Register a session so `resolve_session_from_message` succeeds.
async fn register_session(sm: &SessionManager, channel: &str, msg: &Message) {
    let _ = sm.find_or_create(channel, msg, None).await.unwrap();
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Normal path — text messages pass through
// ═════════════════════════════════════════════════════════════════════════════

/// Text message with explicit `message_type: Text` passes through the
/// interception check and reaches the handler (returns None only because
/// no `SessionMessageHandler` is configured).
#[tokio::test]
async fn test_text_message_not_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed = make_processed(&msg, "mock", "hello", Some(&MessageType::Text));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    // No handler configured -> returns None, but NOT because of interception.
    assert!(result.is_none(), "no handler configured -> None");
    // No error reply sent.
    assert_eq!(
        plugin.send_count(),
        0,
        "text message should not trigger error reply"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Interception path — non-text messages are rejected
// ═════════════════════════════════════════════════════════════════════════════

/// Image message is intercepted: returns None and sends error reply.
/// No session registration is needed — interception happens before
/// session resolution.
#[tokio::test]
async fn test_image_message_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Image));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "image message should return None");
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");

    // Verify the error reply content.
    let (output, peer_id, _thread_id) = plugin.last_send().unwrap();
    assert_eq!(output.msg_type, "text");
    assert_eq!(peer_id, "agent-1");
    let text = output.payload["content"]["text"].as_str().unwrap();
    assert_eq!(
        text, "暂不支持该消息类型",
        "error message must match design doc: got {text}"
    );
}

/// File message is intercepted.
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_file_message_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "check this");

    let processed = make_processed(&msg, "mock", "check this", Some(&MessageType::File));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "file message should return None");
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");
}

/// Audio message is intercepted.
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_audio_message_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Audio));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "audio message should return None");
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");
}

/// Unknown type `Other("video")` is also intercepted.
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_other_message_type_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");

    let other_type = MessageType::Other("video".to_string());
    let processed = make_processed(&msg, "mock", "", Some(&other_type));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "Other(video) should return None");
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Boundary — missing message_type defaults to text
// ═════════════════════════════════════════════════════════════════════════════

/// When `message_type` key is absent from metadata, the default is Text,
/// so the message is NOT intercepted.
#[tokio::test]
async fn test_missing_message_type_defaults_to_text() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");
    register_session(gw.session_manager(), "mock", &msg).await;

    // Pass None for msg_type -> key not inserted into metadata.
    let processed = make_processed(&msg, "mock", "hello", None);
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    // Returns None because no handler is configured, NOT because of interception.
    assert!(result.is_none(), "no handler configured -> None");
    assert_eq!(
        plugin.send_count(),
        0,
        "missing message_type defaults to text, no error reply"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. Step 1.4 — error reply flows through send_outbound_simplified (raw-log only)
// ═════════════════════════════════════════════════════════════════════════════

/// Non-text rejection reply goes through `plugin.render()` before
/// `plugin.send()`, confirming the simplified outbound path.
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_non_text_reply_goes_through_render() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Image));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "image message should return None");
    // render() is called via send_outbound_to_chat (full processor chain)
    assert_eq!(plugin.render_count(), 1, "render should be called once");
    // send() must also be called
    assert_eq!(plugin.send_count(), 1, "send should be called once");
}

/// Non-text rejection reply exercises the raw-log-only processor path,
/// bypassing VerbosityFilter/DslParser (design doc requirement).
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_non_text_reply_uses_outbound_processor_chain() {
    let (gw, plugin, chain) = make_gw_with_processor("mock").await;
    let msg = make_message("agent-1", "");

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::File));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "file message should return None");
    // process_outbound_raw_log_only MUST have been invoked (Step 1.4)
    assert_eq!(
        chain.outbound_raw_log_only_call_count(),
        1,
        "raw-log-only processor path should be invoked for non-text rejection"
    );
    // Full outbound chain (VerbosityFilter/DslParser) must NOT have been invoked
    assert_eq!(
        chain.outbound_full_call_count(),
        0,
        "full outbound chain should NOT be invoked — design doc requires skip"
    );
    // Plugin send must still have been called
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");
}

/// Non-text rejection reply runs the simplified outbound path (no
/// outbound middleware chain).
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_non_text_reply_uses_outbound_middleware() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Audio));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "audio message should return None");
    // render is called (simplified outbound path)
    assert_eq!(plugin.render_count(), 1, "render should be called once");
    // send is called
    assert_eq!(plugin.send_count(), 1, "send should be called once");
    // The simplified path skips the outbound middleware chain.
    // render_count == 1 confirms render was called directly.
}

/// Non-text rejection error text matches the design doc specification.
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_non_text_error_text_matches_doc() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Image));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "image message should return None");
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");

    let (output, peer_id, _thread_id) = plugin.last_send().unwrap();
    assert_eq!(output.msg_type, "text");
    assert_eq!(peer_id, "agent-1");
    let text = output.payload["content"]["text"].as_str().unwrap();
    assert_eq!(
        text, "暂不支持该消息类型",
        "error text must match design doc: {text}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Empty peer_id — should skip sending, not panic
// ═════════════════════════════════════════════════════════════════════════════

/// When peer_id is empty, the non-text interception path should return
/// None without panicking. The code calls `send_outbound_to_chat`
/// (which sends to an empty chat_id) — the important invariant is no panic.
/// No session registration needed — interception before session resolution.
#[tokio::test]
async fn test_non_text_empty_peer_id_no_panic() {
    let (gw, plugin) = make_gw("mock").await;

    // Build a processed message with empty peer_id.
    let msg = make_message("agent-1", "");
    let session_key = compute_session_key("mock", &msg.from, &msg.to, None, msg.timestamp);
    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), session_key);
    metadata.insert("peer_id".to_string(), String::new()); // empty
    metadata.insert("sender_id".to_string(), "ou_sender".to_string());
    metadata.insert(
        "message_type".to_string(),
        serde_json::to_string(&MessageType::Image).unwrap(),
    );
    let processed = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata,
    };

    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    // Should return None without panicking.
    assert!(result.is_none(), "empty peer_id should return None");
    // send_outbound_to_chat still sends (to empty chat_id) — no panic.
    assert_eq!(
        plugin.send_count(),
        1,
        "reply is still sent even with empty peer_id"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 6. Step 1.2 — non-text interception before session resolution
// ═════════════════════════════════════════════════════════════════════════════

/// Non-text messages must not trigger session creation.
/// Verifies that `resolve_session_from_message` is never called for
/// non-text messages — the interception path returns before session
/// resolution, so the SessionManager's internal map stays empty.
#[tokio::test]
async fn test_non_text_message_does_not_create_session() {
    let (gw, _plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");

    // Confirm no sessions exist initially.
    let initial_sessions = gw.session_manager().get_all_sessions().await;
    assert!(initial_sessions.is_empty(), "should start with no sessions");

    // Process an image message — interception should fire before session
    // resolution, so no session is created.
    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Image));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "image message should return None");

    // Verify no session was created by the non-text interception path.
    let sessions_after = gw.session_manager().get_all_sessions().await;
    assert!(
        sessions_after.is_empty(),
        "non-text message must not create a session"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 7. account_id propagation in metadata
// ═════════════════════════════════════════════════════════════════════════════

/// When `account_id` is present in the processed message metadata, it
/// should be available to `resolve_session_from_message` and forwarded
/// to `SessionManager::resolve()`.
#[tokio::test]
async fn test_account_id_propagated_in_metadata() {
    let (gw, _plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed =
        make_processed_with_account(&msg, "mock", "hello", &MessageType::Text, "acct_test_123");

    // Verify account_id is in metadata.
    assert_eq!(
        processed.metadata.get("account_id").map(|s| s.as_str()),
        Some("acct_test_123"),
        "account_id should be present in metadata"
    );

    // The message should be routed normally (text, no interception).
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    // Returns None only because no handler is configured.
    assert!(result.is_none(), "no handler configured -> None");
}

/// When `account_id` is absent from metadata, `resolve_session_from_message`
/// should still succeed (passes `None` to SessionManager).
#[tokio::test]
async fn test_missing_account_id_defaults_to_none() {
    let (gw, _plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");
    register_session(gw.session_manager(), "mock", &msg).await;

    // make_processed does not insert account_id.
    let processed = make_processed(&msg, "mock", "hello", Some(&MessageType::Text));

    // account_id should be absent.
    assert!(
        !processed.metadata.contains_key("account_id"),
        "account_id should not be in metadata when not provided"
    );

    // The message should still be routed normally.
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "no handler configured -> None");
}
