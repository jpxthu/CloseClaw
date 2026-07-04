//! Unit tests for non-text message interception in `handle_inbound_message`.
//!
//! Covers the behavior added in Step 1.1: when `message_type` metadata is
//! not `Text`, the gateway sends an error reply via the plugin and returns
//! `None`, bypassing slash/LLM routing.

use crate::{DmScope, GatewayConfig, HandleResult, Message, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::MessageType;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::processor::ProcessedMessage;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Mock plugin ─────────────────────────────────────────────────────────────

/// Captures `send` invocations so tests can assert on error replies.
struct CapturingPlugin {
    platform: String,
    send_calls: std::sync::Mutex<Vec<(RenderedOutput, String, Option<String>)>>,
}

impl CapturingPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            send_calls: std::sync::Mutex::new(Vec::new()),
        }
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

// ── Test helpers ────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
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
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let capturing: Arc<CapturingPlugin> = Arc::new(CapturingPlugin::new(channel));
    let plugin: Arc<dyn IMPlugin> = capturing.clone() as Arc<dyn IMPlugin>;
    gw.register_plugin(plugin).await;
    (gw, capturing)
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
    let session_key = DmScope::default().compute_session_key(channel, msg, None, msg.timestamp);
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
#[tokio::test]
async fn test_image_message_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");
    register_session(gw.session_manager(), "mock", &msg).await;

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
    assert!(
        text.contains("非文本"),
        "error message should mention non-text: got {text}"
    );
}

/// File message is intercepted.
#[tokio::test]
async fn test_file_message_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "check this");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed = make_processed(&msg, "mock", "check this", Some(&MessageType::File));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "file message should return None");
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");
}

/// Audio message is intercepted.
#[tokio::test]
async fn test_audio_message_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Audio));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "audio message should return None");
    assert_eq!(plugin.send_count(), 1, "error reply should be sent");
}

/// Unknown type `Other("video")` is also intercepted.
#[tokio::test]
async fn test_other_message_type_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");
    register_session(gw.session_manager(), "mock", &msg).await;

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
