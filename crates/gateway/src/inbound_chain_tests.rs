//! Step 1.5 — Unit tests for InboundChainInput field propagation.
//!
//! Verifies that fields added in Step 1.1 (thread_id, message_type,
//! media_refs, quoted_message) survive the InboundChainInput →
//! process_inbound_chain → ProcessedMessage pipeline and are accessible
//! in Gateway metadata.

use crate::{DmScope, GatewayConfig, InboundChainInput, SessionManager};
use closeclaw_common::im_plugin::{MediaRef, MessageType};
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Test helpers ─────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

fn make_gw() -> crate::Gateway {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    crate::Gateway::new(config, sm)
}

/// Build a fully-populated InboundChainInput for the normal-path test.
fn full_chain_input() -> InboundChainInput {
    InboundChainInput {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: "hello world".into(),
        message_id: "msg_001".into(),
        timestamp_ms: 1_700_000_000_000,
        account_id: Some("acct_foo".into()),
        thread_id: Some("ot_thread_abc".into()),
        message_type: MessageType::Text,
        media_refs: vec![MediaRef {
            key: "img_key_1".into(),
            url: "https://example.com/img1.png".into(),
        }],
        quoted_message: Some("original message".into()),
    }
}

/// Build an InboundChainInput with all optional fields at defaults.
fn default_chain_input() -> InboundChainInput {
    InboundChainInput {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: "hello".into(),
        message_id: "msg_002".into(),
        timestamp_ms: 1_700_000_000_000,
        account_id: Some("acct_foo".into()),
        thread_id: None,
        message_type: MessageType::Text,
        media_refs: Vec::new(),
        quoted_message: None,
    }
}

/// Build an InboundChainInput for a non-text (image) message.
fn image_chain_input() -> InboundChainInput {
    InboundChainInput {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: String::new(),
        message_id: "msg_003".into(),
        timestamp_ms: 1_700_000_000_000,
        account_id: Some("acct_foo".into()),
        thread_id: Some("ot_thread_img".into()),
        message_type: MessageType::Image,
        media_refs: vec![MediaRef {
            key: "img_k_99".into(),
            url: "https://example.com/photo.jpg".into(),
        }],
        quoted_message: None,
    }
}

/// Build an InboundChainInput for a file message.
fn file_chain_input() -> InboundChainInput {
    InboundChainInput {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: "check this file".into(),
        message_id: "msg_004".into(),
        timestamp_ms: 1_700_000_000_000,
        account_id: Some("acct_foo".into()),
        thread_id: None,
        message_type: MessageType::File,
        media_refs: vec![MediaRef {
            key: "file_k_10".into(),
            url: "https://example.com/doc.pdf".into(),
        }],
        quoted_message: Some("see attached".into()),
    }
}

/// Build an InboundChainInput for an audio message.
fn audio_chain_input() -> InboundChainInput {
    InboundChainInput {
        platform: "feishu".into(),
        sender_id: "ou_sender1".into(),
        peer_id: "oc_chat1".into(),
        content: String::new(),
        message_id: "msg_005".into(),
        timestamp_ms: 1_700_000_000_000,
        account_id: Some("acct_foo".into()),
        thread_id: Some("ot_audio_thread".into()),
        message_type: MessageType::Audio,
        media_refs: vec![MediaRef {
            key: "audio_k_1".into(),
            url: "https://example.com/voice.m4a".into(),
        }],
        quoted_message: None,
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Normal path: all fields propagated
// ═════════════════════════════════════════════════════════════════════════════

/// Construct InboundChainInput with all fields populated and verify
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

    // message_type serialized as JSON string.
    let mt = result.metadata.get("message_type").map(|s| s.as_str());
    let deserialized: MessageType = serde_json::from_str(mt.unwrap()).unwrap();
    assert_eq!(deserialized, MessageType::Text);

    // media_refs serialized as JSON array.
    let mr = result.metadata.get("media_refs").map(|s| s.as_str());
    let refs: Vec<MediaRef> = serde_json::from_str(mr.unwrap()).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "img_key_1");
    assert_eq!(refs[0].url, "https://example.com/img1.png");

    // quoted_message in metadata.
    let qm = result.metadata.get("quoted_message").map(|s| s.as_str());
    assert_eq!(qm, Some("original message"));
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
async fn test_defaults_message_type_text() {
    let gw = make_gw();
    let input = default_chain_input();

    let result = gw.process_inbound_chain(&input).await;
    let mt = result.metadata.get("message_type").unwrap();
    let deserialized: MessageType = serde_json::from_str(mt).unwrap();
    assert_eq!(
        deserialized,
        MessageType::Text,
        "default message_type should be Text"
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

#[tokio::test]
async fn test_defaults_quoted_message_none() {
    let gw = make_gw();
    let input = default_chain_input();

    let result = gw.process_inbound_chain(&input).await;
    assert!(
        !result.metadata.contains_key("quoted_message"),
        "quoted_message key absent when None"
    );
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

    // message_type deserializes to Image.
    let mt = result.metadata.get("message_type").unwrap();
    let deserialized: MessageType = serde_json::from_str(mt).unwrap();
    assert_eq!(deserialized, MessageType::Image);

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

/// File message: message_type=File, thread_id absent, quoted_message present.
#[tokio::test]
async fn test_file_message_type_propagated() {
    let gw = make_gw();
    let input = file_chain_input();

    let result = gw.process_inbound_chain(&input).await;

    let mt = result.metadata.get("message_type").unwrap();
    let deserialized: MessageType = serde_json::from_str(mt).unwrap();
    assert_eq!(deserialized, MessageType::File);

    let mr = result.metadata.get("media_refs").unwrap();
    let refs: Vec<MediaRef> = serde_json::from_str(mr).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "file_k_10");

    assert!(
        !result.metadata.contains_key("thread_id"),
        "file_chain_input has no thread_id"
    );

    let qm = result.metadata.get("quoted_message").map(|s| s.as_str());
    assert_eq!(qm, Some("see attached"));
}

/// Audio message: message_type=Audio, thread_id present.
#[tokio::test]
async fn test_audio_message_type_propagated() {
    let gw = make_gw();
    let input = audio_chain_input();

    let result = gw.process_inbound_chain(&input).await;

    let mt = result.metadata.get("message_type").unwrap();
    let deserialized: MessageType = serde_json::from_str(mt).unwrap();
    assert_eq!(deserialized, MessageType::Audio);

    let mr = result.metadata.get("media_refs").unwrap();
    let refs: Vec<MediaRef> = serde_json::from_str(mr).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].url, "https://example.com/voice.m4a");

    assert_eq!(
        result.metadata.get("thread_id").map(|s| s.as_str()),
        Some("ot_audio_thread")
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Metadata field types — round-trip serialization
// ═════════════════════════════════════════════════════════════════════════════

/// Verify that message_type round-trips correctly through the metadata
/// as a JSON string (serde_json::to_string → serde_json::from_str).
#[test]
fn test_message_type_metadata_roundtrip() {
    let cases = vec![
        (MessageType::Text, r#""text""#),
        (MessageType::Image, r#""image""#),
        (MessageType::File, r#""file""#),
        (MessageType::Audio, r#""audio""#),
        (MessageType::Other("custom".into()), r#""custom""#),
    ];
    for (mt, expected_json) in cases {
        let serialized = serde_json::to_string(&mt).unwrap();
        assert_eq!(serialized, expected_json);
        let deserialized: MessageType = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, mt);
    }
}

/// Verify that media_refs round-trips correctly through metadata JSON.
#[test]
fn test_media_refs_metadata_roundtrip() {
    let refs = vec![
        MediaRef {
            key: "k1".into(),
            url: "https://a.com/1".into(),
        },
        MediaRef {
            key: "k2".into(),
            url: "https://b.com/2".into(),
        },
    ];
    let json = serde_json::to_string(&refs).unwrap();
    let back: Vec<MediaRef> = serde_json::from_str(&json).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].key, "k1");
    assert_eq!(back[1].url, "https://b.com/2");
}
