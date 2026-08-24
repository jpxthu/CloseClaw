//! Step 1.5 — Unit tests for NormalizedMessage field propagation.
//!
//! Verifies that fields added in Step 1.1 (thread_id, media_refs)
//! survive the NormalizedMessage →
//! process_inbound_chain → ProcessedMessage pipeline and are accessible
//! in Gateway metadata.
//!
//! Note: `message_type` is injected by the Processor Chain (SessionRouter),
//! not by Gateway's `build_extra_metadata`. These tests use the no-registry
//! fallback path, so `message_type` is NOT expected in metadata.

use crate::{GatewayConfig, SessionManager};
use closeclaw_common::im_plugin::{MediaRef, MessageType, NormalizedMessage};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Test helpers ─────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_gw() -> crate::Gateway {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    crate::Gateway::new(config, sm)
}

/// Build a fully-populated NormalizedMessage for the normal-path test.
fn full_chain_input() -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: "hello world".into(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: vec![MediaRef {
            key: "img_key_1".into(),
            url: "https://example.com/img1.png".into(),
        }],
        thread_id: Some("ot_thread_abc".into()),
        account_id: "acct_foo".into(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: "msg_001".into(),
    }
}

/// Build a NormalizedMessage with all optional fields at defaults.
fn default_chain_input() -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: "hello".into(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Text,
        media_refs: Vec::new(),
        thread_id: None,
        account_id: "acct_foo".into(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: "msg_002".into(),
    }
}

/// Build a NormalizedMessage for a non-text (image) message.
fn image_chain_input() -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: String::new(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Image,
        media_refs: vec![MediaRef {
            key: "img_k_99".into(),
            url: "https://example.com/img99.png".into(),
        }],
        thread_id: Some("ot_thread_img".into()),
        account_id: "acct_foo".into(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: "msg_003".into(),
    }
}

/// Build a NormalizedMessage for a file message.
fn file_chain_input() -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: "check this file".into(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::File,
        media_refs: vec![MediaRef {
            key: "file_k_10".into(),
            url: "https://example.com/file10.pdf".into(),
        }],
        thread_id: None,
        account_id: "acct_foo".into(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: "msg_004".into(),
    }
}

/// Build a NormalizedMessage for an audio message.
fn audio_chain_input() -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: String::new(),
        timestamp: 1_700_000_000_000,
        message_type: MessageType::Audio,
        media_refs: vec![MediaRef {
            key: "audio_k_5".into(),
            url: "https://example.com/voice.m4a".into(),
        }],
        thread_id: Some("ot_audio_thread".into()),
        account_id: "acct_foo".into(),
        chat_name: String::new(),
        trace_id: String::new(),
        message_id: "msg_005".into(),
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Normal path: all fields propagated
// ═════════════════════════════════════════════════════════════════════════════

/// Construct NormalizedMessage with all fields populated and verify
/// that process_inbound_chain places them into ProcessedMessage.metadata.
#[tokio::test]
async fn test_all_fields_propagated_no_registry() {
    let gw = make_gw();
    let input = full_chain_input();

    let result = gw.process_inbound_chain(&input).await;

    // Content preserved.
    assert_eq!(result.text_content(), Some("hello world"));

    // thread_id in metadata.
    let thread = result.metadata.get("thread_id").map(|s| s.as_str());
    assert_eq!(thread, Some("ot_thread_abc"));

    // message_type is NOT in extra metadata (injected by Processor Chain, not Gateway).
    assert!(
        !result.metadata.contains_key("message_type"),
        "message_type should not be in extra metadata — injected by Processor Chain"
    );

    // media_refs serialized as JSON array.
    let mr = result.metadata.get("media_refs").map(|s| s.as_str());
    let refs: Vec<MediaRef> = serde_json::from_str(mr.unwrap()).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "img_key_1");
    assert_eq!(refs[0].url, "https://example.com/img1.png");
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. thread_id passthrough
// ═════════════════════════════════════════════════════════════════════════════

/// thread_id is preserved through the pipeline when present.
#[tokio::test]
async fn test_thread_id_passthrough() {
    let gw = make_gw();
    let input = full_chain_input();
    assert_eq!(input.thread_id.as_deref(), Some("ot_thread_abc"));

    let result = gw.process_inbound_chain(&input).await;
    assert_eq!(
        result.metadata.get("thread_id").map(|s| s.as_str()),
        Some("ot_thread_abc"),
        "thread_id must survive process_inbound_chain"
    );
}

/// thread_id absent → not inserted into metadata.
#[tokio::test]
async fn test_thread_id_absent_not_in_metadata() {
    let gw = make_gw();
    let input = default_chain_input();
    assert!(input.thread_id.is_none());

    let result = gw.process_inbound_chain(&input).await;
    assert!(
        !result.metadata.contains_key("thread_id"),
        "thread_id key should not be present when input.thread_id is None"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Default values
// ═════════════════════════════════════════════════════════════════════════════

/// When optional fields are at defaults (None / empty Vec), metadata
/// should reflect those defaults sensibly.
#[tokio::test]
async fn test_defaults_thread_id_none() {
    let gw = make_gw();
    let input = default_chain_input();

    let result = gw.process_inbound_chain(&input).await;
    assert!(
        !result.metadata.contains_key("thread_id"),
        "no thread_id key when input is None"
    );
}

#[tokio::test]
async fn test_defaults_message_type_not_in_extra_metadata() {
    let gw = make_gw();
    let input = default_chain_input();

    let result = gw.process_inbound_chain(&input).await;
    // message_type is injected by Processor Chain (SessionRouter), not Gateway.
    assert!(
        !result.metadata.contains_key("message_type"),
        "message_type should not be in extra metadata — injected by Processor Chain"
    );
}

#[tokio::test]
async fn test_defaults_media_refs_empty() {
    let gw = make_gw();
    let input = default_chain_input();

    let result = gw.process_inbound_chain(&input).await;
    let mr = result.metadata.get("media_refs").unwrap();
    let refs: Vec<MediaRef> = serde_json::from_str(mr).unwrap();
    assert!(refs.is_empty(), "default media_refs should be empty array");
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. Non-text messages
// ═════════════════════════════════════════════════════════════════════════════

/// Image message: message_type=Image, media_refs has entries.
#[tokio::test]
async fn test_image_message_type_propagated() {
    let gw = make_gw();
    let input = image_chain_input();

    let result = gw.process_inbound_chain(&input).await;

    // message_type is NOT in extra metadata (injected by Processor Chain, not Gateway).
    assert!(
        !result.metadata.contains_key("message_type"),
        "message_type should not be in extra metadata — injected by Processor Chain"
    );

    // media_refs non-empty.
    let mr = result.metadata.get("media_refs").unwrap();
    let refs: Vec<MediaRef> = serde_json::from_str(mr).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "img_k_99");

    // thread_id propagated.
    assert_eq!(
        result.metadata.get("thread_id").map(|s| s.as_str()),
        Some("ot_thread_img")
    );

    // Content may be empty for image messages (design doc allows it).
    assert_eq!(result.text_content(), Some(""));
}

/// File message: message_type=File, thread_id absent.
#[tokio::test]
async fn test_file_message_type_propagated() {
    let gw = make_gw();
    let input = file_chain_input();

    let result = gw.process_inbound_chain(&input).await;

    // message_type is NOT in extra metadata (injected by Processor Chain, not Gateway).
    assert!(
        !result.metadata.contains_key("message_type"),
        "message_type should not be in extra metadata — injected by Processor Chain"
    );

    let mr = result.metadata.get("media_refs").unwrap();
    let refs: Vec<MediaRef> = serde_json::from_str(mr).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "file_k_10");

    assert!(
        !result.metadata.contains_key("thread_id"),
        "file_chain_input has no thread_id"
    );
}

/// Audio message: message_type=Audio, thread_id present.
#[tokio::test]
async fn test_audio_message_type_propagated() {
    let gw = make_gw();
    let input = audio_chain_input();

    let result = gw.process_inbound_chain(&input).await;

    // message_type is NOT in extra metadata (injected by Processor Chain, not Gateway).
    assert!(
        !result.metadata.contains_key("message_type"),
        "message_type should not be in extra metadata — injected by Processor Chain"
    );

    let mr = result.metadata.get("media_refs").unwrap();
    let refs: Vec<MediaRef> = serde_json::from_str(mr).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].url, "https://example.com/voice.m4a");

    assert_eq!(
        result.metadata.get("thread_id").map(|s| s.as_str()),
        Some("ot_audio_thread")
    );
}
