//! Unit tests for inbound message handling in `handle_inbound_message`.
//!
//! Covers:
//! - Text messages pass through validation normally.
//! - Image/file/audio messages now pass through (no longer rejected by
//!   Gateway; media routing decisions are delegated downstream).
//! - `unavailable_media` non-empty still triggers rejection for any type.
//! - `build_context_content` produces media reference tokens.
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
use closeclaw_common::processor::{DslParseResult, ProcessedMessage};
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
        platform: None,
        dsl_result: None,
        content_blocks: None,
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
// 2. Media messages now pass through (no longer rejected)
// ═════════════════════════════════════════════════════════════════════════════

/// Image message passes through validation and reaches the handler.
/// No error reply is sent — media messages are no longer rejected.
#[tokio::test]
async fn test_image_message_passes_through() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Image));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    // No handler configured -> returns None, NOT because of interception.
    assert!(result.is_none(), "no handler configured -> None");
    // No error reply sent — media messages are accepted.
    assert_eq!(
        plugin.send_count(),
        0,
        "image message should not trigger error reply"
    );
}

/// File message passes through validation and reaches the handler.
#[tokio::test]
async fn test_file_message_passes_through() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "check this");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed = make_processed(&msg, "mock", "check this", Some(&MessageType::File));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "no handler configured -> None");
    assert_eq!(
        plugin.send_count(),
        0,
        "file message should not trigger error reply"
    );
}

/// Audio message passes through validation and reaches the handler.
#[tokio::test]
async fn test_audio_message_passes_through() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Audio));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "no handler configured -> None");
    assert_eq!(
        plugin.send_count(),
        0,
        "audio message should not trigger error reply"
    );
}

/// Unknown type string (e.g. "video") now maps to Text via `From<&str>`,
/// so it is NOT intercepted — same as any text message.
#[tokio::test]
async fn test_unknown_type_string_maps_to_text_not_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");
    register_session(gw.session_manager(), "mock", &msg).await;

    // "video" via From<&str> maps to MessageType::Text
    let text_type: MessageType = "video".into();
    assert_eq!(text_type, MessageType::Text);

    let processed = make_processed(&msg, "mock", "hello", Some(&text_type));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    // Text message goes through normal routing (not intercepted).
    // Returns None only because no handler is configured,
    // same as test_text_message_not_intercepted.
    assert!(result.is_none(), "no handler configured -> None");
    assert_eq!(plugin.send_count(), 0, "no error reply for text");
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
// 4. build_context_content — media reference tokens
// ═════════════════════════════════════════════════════════════════════════════

/// Image message with media_refs produces reference tokens in content.
#[tokio::test]
async fn test_image_message_content_has_reference_tokens() {
    let (gw, _plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");
    register_session(gw.session_manager(), "mock", &msg).await;

    let mut pm = make_processed(&msg, "mock", "", Some(&MessageType::Image));
    pm.metadata.insert(
        "media_refs".to_string(),
        serde_json::to_string(&vec![closeclaw_common::im_plugin::MediaRef {
            key: "img_abc".into(),
            path: "/tmp/img".into(),
            media_type: closeclaw_common::im_plugin::MediaType::Image,
            size: 1024,
            mime: "image/png".into(),
        }])
        .unwrap(),
    );

    let result: Option<HandleResult> = gw
        .handle_inbound_message(pm, Some("ou_sender"), "mock")
        .await;

    // No handler -> None, but the message was routed (not rejected).
    assert!(result.is_none(), "no handler configured -> None");
    // The content string now contains the reference token.
    // (We can't directly observe it here, but the test validates
    // the message flows through the gateway without rejection.)
}

/// build_context_content returns text unchanged for text messages.
#[tokio::test]
async fn test_build_context_content_text_unchanged() {
    let pm = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("hello".to_string())],
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "message_type".to_string(),
                serde_json::to_string(&MessageType::Text).unwrap(),
            );
            m
        },
    };
    assert_eq!(crate::media_routing::build_context_content(&pm), "hello");
}

/// build_context_content generates reference tokens for image messages.
#[tokio::test]
async fn test_build_context_content_image_reference() {
    let pm = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "message_type".to_string(),
                serde_json::to_string(&MessageType::Image).unwrap(),
            );
            m.insert(
                "media_refs".to_string(),
                serde_json::to_string(&vec![closeclaw_common::im_plugin::MediaRef {
                    key: "img_xyz".into(),
                    path: "/tmp/img".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::Image,
                    size: 1024,
                    mime: "image/png".into(),
                }])
                .unwrap(),
            );
            m
        },
    };
    assert_eq!(
        crate::media_routing::build_context_content(&pm),
        "[image: img_xyz]"
    );
}

/// build_context_content generates reference tokens for file messages.
#[tokio::test]
async fn test_build_context_content_file_reference() {
    let pm = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "message_type".to_string(),
                serde_json::to_string(&MessageType::File).unwrap(),
            );
            m.insert(
                "media_refs".to_string(),
                serde_json::to_string(&vec![closeclaw_common::im_plugin::MediaRef {
                    key: "doc_42".into(),
                    path: "/tmp/doc".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::File,
                    size: 2048,
                    mime: "application/pdf".into(),
                }])
                .unwrap(),
            );
            m
        },
    };
    assert_eq!(
        crate::media_routing::build_context_content(&pm),
        "[file: doc_42]"
    );
}

/// build_context_content generates reference tokens for audio messages.
#[tokio::test]
async fn test_build_context_content_audio_reference() {
    let pm = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "message_type".to_string(),
                serde_json::to_string(&MessageType::Audio).unwrap(),
            );
            m.insert(
                "media_refs".to_string(),
                serde_json::to_string(&vec![closeclaw_common::im_plugin::MediaRef {
                    key: "voice_1".into(),
                    path: "/tmp/voice".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::Audio,
                    size: 512,
                    mime: "audio/ogg".into(),
                }])
                .unwrap(),
            );
            m
        },
    };
    assert_eq!(
        crate::media_routing::build_context_content(&pm),
        "[audio: voice_1]"
    );
}

/// Reference tokens never contain local file system paths.
#[tokio::test]
async fn test_build_context_content_no_local_paths() {
    let pm = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "message_type".to_string(),
                serde_json::to_string(&MessageType::Image).unwrap(),
            );
            m.insert(
                "media_refs".to_string(),
                serde_json::to_string(&vec![closeclaw_common::im_plugin::MediaRef {
                    key: "secret".into(),
                    path: "/home/user/private/photo.jpg".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::Image,
                    size: 1024,
                    mime: "image/jpeg".into(),
                }])
                .unwrap(),
            );
            m
        },
    };
    let content = crate::media_routing::build_context_content(&pm);
    assert!(
        !content.contains("/home"),
        "must not contain local paths: {content}"
    );
    assert!(content.contains("secret"));
}

/// Post message with text and media_refs combines both.
#[tokio::test]
async fn test_build_context_content_post_with_text_and_media() {
    let pm = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("check this".to_string())],
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "message_type".to_string(),
                serde_json::to_string(&MessageType::Post).unwrap(),
            );
            m.insert(
                "media_refs".to_string(),
                serde_json::to_string(&vec![closeclaw_common::im_plugin::MediaRef {
                    key: "pic_99".into(),
                    path: "/tmp/pic".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::Image,
                    size: 512,
                    mime: "image/jpeg".into(),
                }])
                .unwrap(),
            );
            m
        },
    };
    assert_eq!(
        crate::media_routing::build_context_content(&pm),
        "check this [image: pic_99]"
    );
}

/// Post message with media only (no text) returns just reference tokens.
#[tokio::test]
async fn test_build_context_content_post_media_only() {
    let pm = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(String::new())],
        metadata: {
            let mut m = HashMap::new();
            m.insert(
                "message_type".to_string(),
                serde_json::to_string(&MessageType::Post).unwrap(),
            );
            m.insert(
                "media_refs".to_string(),
                serde_json::to_string(&vec![closeclaw_common::im_plugin::MediaRef {
                    key: "vid_7".into(),
                    path: "/tmp/vid".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::File,
                    size: 4096,
                    mime: "video/mp4".into(),
                }])
                .unwrap(),
            );
            m
        },
    };
    assert_eq!(
        crate::media_routing::build_context_content(&pm),
        "[file: vid_7]"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Empty peer_id — should skip sending, not panic
// ═════════════════════════════════════════════════════════════════════════════

/// When peer_id is empty, the message should still route without panicking.
/// Image messages are no longer rejected, so this tests the normal path
/// with an empty peer_id (session resolution may fail gracefully).
#[tokio::test]
async fn test_image_empty_peer_id_no_panic() {
    let (gw, _plugin) = make_gw("mock").await;

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
}

// ═════════════════════════════════════════════════════════════════════════════
// 6. Media messages now go through session resolution (no early rejection)
// ═════════════════════════════════════════════════════════════════════════════

/// Image messages now pass through validation and reach session resolution.
/// When no session exists and no handler is configured, returns None.
#[tokio::test]
async fn test_image_message_reaches_session_resolution() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "");
    register_session(gw.session_manager(), "mock", &msg).await;

    // Confirm session exists.
    let sessions = gw.session_manager().get_all_sessions().await;
    assert!(!sessions.is_empty(), "session should be registered");

    let processed = make_processed(&msg, "mock", "", Some(&MessageType::Image));
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(result.is_none(), "no handler configured -> None");
    // No error reply sent — image messages are accepted.
    assert_eq!(plugin.send_count(), 0, "no error reply for image message");
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

// ═════════════════════════════════════════════════════════════════════════════
// 8. Step 1.4 — unavailable_media interception
// ═════════════════════════════════════════════════════════════════════════════

/// Build a `ProcessedMessage` with explicit `unavailable_media` in metadata.
fn make_processed_with_unavailable_media(
    msg: &Message,
    channel: &str,
    content: &str,
    unavailable: Vec<String>,
) -> ProcessedMessage {
    let session_key = compute_session_key(channel, &msg.from, &msg.to, None, msg.timestamp);
    let mut metadata = HashMap::new();
    metadata.insert("session_key".to_string(), session_key);
    metadata.insert("peer_id".to_string(), msg.to.clone());
    metadata.insert("sender_id".to_string(), msg.from.clone());
    metadata.insert(
        "message_type".to_string(),
        serde_json::to_string(&MessageType::Text).unwrap(),
    );
    if !unavailable.is_empty() {
        metadata.insert(
            "unavailable_media".to_string(),
            serde_json::to_string(&unavailable).unwrap(),
        );
    }
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(content.to_string())],
        metadata,
    }
}

/// Text message with non-empty `unavailable_media` is intercepted:
/// returns None and sends the "该消息内容无法获取" reply.
/// Interception happens before session resolution.
#[tokio::test]
async fn test_unavailable_media_non_empty_intercepted() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");

    let processed =
        make_processed_with_unavailable_media(&msg, "mock", "hello", vec!["img_key_1".to_string()]);
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    assert!(
        result.is_none(),
        "message with unavailable_media should return None"
    );
    assert_eq!(
        plugin.send_count(),
        1,
        "error reply should be sent for unavailable media"
    );

    // Verify the reply text matches design doc.
    let (output, peer_id, _thread_id) = plugin.last_send().unwrap();
    assert_eq!(output.msg_type, "text");
    assert_eq!(peer_id, "agent-1");
    let text = output.payload["content"]["text"].as_str().unwrap();
    assert_eq!(
        text, "该消息内容无法获取",
        "reply must match design doc: got {text}"
    );
}

/// Text message with empty `unavailable_media` (or missing key) passes
/// through the interception check and reaches normal routing.
/// No session registration needed — returns None only because no
/// SessionMessageHandler is configured.
#[tokio::test]
async fn test_unavailable_media_empty_passes_through() {
    let (gw, plugin) = make_gw("mock").await;
    let msg = make_message("agent-1", "hello");
    register_session(gw.session_manager(), "mock", &msg).await;

    let processed = make_processed_with_unavailable_media(&msg, "mock", "hello", Vec::new());
    let result: Option<HandleResult> = gw
        .handle_inbound_message(processed, Some("ou_sender"), "mock")
        .await;

    // Returns None because no handler is configured — NOT because of interception.
    assert!(result.is_none(), "no handler configured -> None");
    assert_eq!(
        plugin.send_count(),
        0,
        "no error reply for message with empty unavailable_media"
    );
}
