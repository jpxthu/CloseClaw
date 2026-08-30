//! Unit tests for NormalizedMessage and related IM plugin types.

use crate::im_plugin::{MediaRef, MediaType, MessageType, NormalizedMessage};
use serde_json;

fn make_normalized(account_id: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "feishu".into(),
        sender_id: "ou_111".into(),
        peer_id: "oc_chat".into(),
        content: "hello".into(),
        timestamp: 1700000000000,
        message_type: MessageType::Text,
        media_refs: vec![],
        reply_ref: None,
        unavailable_media: Vec::new(),
        thread_id: None,
        account_id: account_id.into(),
        ..Default::default()
    }
}

/// Helper: assert MessageType serializes to expected JSON and deserializes back.
fn assert_mt_roundtrip(mt: &MessageType, expected_json: &str) {
    let json = serde_json::to_string(mt).unwrap();
    assert_eq!(json, expected_json, "serialization mismatch for {:?}", mt);
    let de: MessageType = serde_json::from_str(&json).unwrap();
    assert_eq!(mt, &de, "deserialization round-trip failed for {:?}", mt);
}

#[test]
fn test_normalized_account_id_is_string_not_option() {
    let msg = make_normalized("acct_1");
    assert_eq!(msg.account_id, "acct_1");
}

#[test]
fn test_normalized_account_id_empty_string_allowed() {
    let msg = make_normalized("");
    assert!(msg.account_id.is_empty());
}

#[test]
fn test_normalized_no_card_action_field() {
    let msg = make_normalized("a");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("card_action"));
}

#[test]
fn test_normalized_message_type_defaults_to_text() {
    let json = r#"{
        "platform": "p",
        "sender_id": "s",
        "peer_id": "r",
        "content": "x",
        "timestamp": 0
    }"#;
    let msg: NormalizedMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.message_type, MessageType::Text);
}

#[test]
fn test_normalized_roundtrip() {
    let mut msg = make_normalized("tenant_42");
    msg.message_type = MessageType::Image;
    msg.media_refs = vec![MediaRef {
        key: "file_abc".into(),
        path: "/media/file_abc".into(),
        media_type: MediaType::Image,
        size: 0,
        mime: "image/png".into(),
    }];
    msg.thread_id = Some("t_99".into());

    let json = serde_json::to_string(&msg).unwrap();
    let de: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(de.account_id, "tenant_42");
    assert_eq!(de.message_type, MessageType::Image);
    assert_eq!(de.media_refs.len(), 1);
    assert_eq!(de.media_refs[0].key, "file_abc");
    assert_eq!(de.thread_id.as_deref(), Some("t_99"));
}

// ---- MessageType serialization round-trip tests ----

#[test]
fn test_message_type_text_roundtrip() {
    assert_mt_roundtrip(&MessageType::Text, r#""text""#);
}

#[test]
fn test_message_type_image_roundtrip() {
    assert_mt_roundtrip(&MessageType::Image, r#""image""#);
}

#[test]
fn test_message_type_file_roundtrip() {
    assert_mt_roundtrip(&MessageType::File, r#""file""#);
}

#[test]
fn test_message_type_audio_roundtrip() {
    assert_mt_roundtrip(&MessageType::Audio, r#""audio""#);
}

/// Unknown string deserializes to Text (no Other variant).
#[test]
fn test_message_type_deserialize_unknown_string() {
    let mt: MessageType = serde_json::from_str(r#""unknown_type""#).unwrap();
    assert_eq!(mt, MessageType::Text);
}

/// Unknown string via From<&str> maps to Text (no Other variant).
#[test]
fn test_message_type_from_str_unknown_maps_to_text() {
    let mt: MessageType = "video".into();
    assert_eq!(
        mt,
        MessageType::Text,
        "unknown type 'video' should map to Text"
    );

    let mt2: MessageType = "sticker".into();
    assert_eq!(
        mt2,
        MessageType::Text,
        "unknown type 'sticker' should map to Text"
    );

    let mt3: MessageType = "post".into();
    assert_eq!(
        mt3,
        MessageType::Text,
        "'post' (Feishu rich text) should map to Text"
    );
}

/// Known strings map to their respective variants.
#[test]
fn test_message_type_from_str_known_variants() {
    let mt_text: MessageType = "text".into();
    assert_eq!(mt_text, MessageType::Text);

    let mt_image: MessageType = "image".into();
    assert_eq!(mt_image, MessageType::Image);

    let mt_file: MessageType = "file".into();
    assert_eq!(mt_file, MessageType::File);

    let mt_audio: MessageType = "audio".into();
    assert_eq!(mt_audio, MessageType::Audio);
}

#[test]
fn test_message_type_default_is_text() {
    let mt = MessageType::default();
    assert_eq!(mt, MessageType::Text);
}

#[test]
fn test_message_type_in_normalized_message_roundtrip() {
    let mut msg = make_normalized("a");
    msg.message_type = MessageType::Audio;
    let json = serde_json::to_string(&msg).unwrap();
    let de: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(de.message_type, MessageType::Audio);
}

#[test]
fn test_normalized_optional_fields_absent() {
    let json = r#"{
        "platform": "d",
        "sender_id": "1",
        "peer_id": "2",
        "content": "c",
        "timestamp": 0,
        "account_id": "x"
    }"#;
    let msg: NormalizedMessage = serde_json::from_str(json).unwrap();
    assert!(msg.media_refs.is_empty());
    assert!(msg.thread_id.is_none());
}

// ===========================================================================
// Gap 4: IMPlugin trait default delegation tests
// ===========================================================================

use crate::im_plugin::{AdapterError, IMPlugin, StreamingOutput};
use crate::processor::{ContentBlock, StreamEvent};
use crate::streaming::{DefaultStreamingRenderer, StreamingRenderer};
use std::sync::Mutex;

/// A mock plugin that returns `Some(renderer)` from `streaming_renderer()`.
/// Does NOT override `handle_stream_event`, `flush_stream`, or
/// `check_stream_timeout` — relies on the default trait implementations.
struct DelegatingPlugin {
    renderer: Mutex<DefaultStreamingRenderer>,
}

impl DelegatingPlugin {
    fn new() -> Self {
        Self {
            renderer: Mutex::new(DefaultStreamingRenderer::new()),
        }
    }
}

#[async_trait::async_trait]
impl IMPlugin for DelegatingPlugin {
    fn platform(&self) -> &str {
        "delegating"
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
        _dsl_result: Option<&crate::processor::DslParseResult>,
    ) -> crate::im_plugin::RenderedOutput {
        crate::im_plugin::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::Null,
        }
    }

    async fn send(
        &self,
        _output: &crate::im_plugin::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    fn streaming_renderer(&self) -> Option<&Mutex<DefaultStreamingRenderer>> {
        Some(&self.renderer)
    }
    // NOTE: handle_stream_event, flush_stream, check_stream_timeout
    // are NOT overridden — they use the default trait implementations.
}

/// A mock plugin that returns `None` from `streaming_renderer()`.
struct NoRendererPlugin;

#[async_trait::async_trait]
impl IMPlugin for NoRendererPlugin {
    fn platform(&self) -> &str {
        "no_renderer"
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
        _dsl_result: Option<&crate::processor::DslParseResult>,
    ) -> crate::im_plugin::RenderedOutput {
        crate::im_plugin::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::Null,
        }
    }

    async fn send(
        &self,
        _output: &crate::im_plugin::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    // streaming_renderer() returns None (default).
    // handle_stream_event, flush_stream, check_stream_timeout
    // use default trait implementations.
}

/// Gap 4: Default `handle_stream_event` delegates to `streaming_renderer()`
/// when it returns `Some`.
#[test]
fn test_default_handle_stream_event_delegates_to_renderer() {
    let plugin = DelegatingPlugin::new();
    let event = StreamEvent::BlockStart {
        index: 0,
        block_type: crate::processor::ContentBlockType::Text,
    };
    let out = plugin.handle_stream_event(event);
    // The default implementation delegates to the renderer, which processes
    // BlockStart and returns an empty output (no text completed yet).
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());
}

/// Gap 4: Default `handle_stream_event` returns empty when
/// `streaming_renderer()` returns `None`.
#[test]
fn test_default_handle_stream_event_returns_empty_when_no_renderer() {
    let plugin = NoRendererPlugin;
    let event = StreamEvent::BlockStart {
        index: 0,
        block_type: crate::processor::ContentBlockType::Text,
    };
    let out = plugin.handle_stream_event(event);
    assert_eq!(out, StreamingOutput::default());
}

/// Gap 4: Default `flush_stream` delegates to renderer when available.
#[test]
fn test_default_flush_stream_delegates_to_renderer() {
    let plugin = DelegatingPlugin::new();
    // Feed some partial text via the renderer, then flush.
    {
        let mut r = plugin.renderer.lock().unwrap();
        r.handle_event(StreamEvent::BlockStart {
            index: 0,
            block_type: crate::processor::ContentBlockType::Text,
        });
        r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: crate::processor::ContentDelta::Text {
                text: "partial".to_string(),
            },
        });
    }
    let out = plugin.flush_stream();
    assert_eq!(out.text_messages, vec!["partial"]);
}

/// Gap 4: Default `flush_stream` returns empty when no renderer.
#[test]
fn test_default_flush_stream_returns_empty_when_no_renderer() {
    let plugin = NoRendererPlugin;
    let out = plugin.flush_stream();
    assert_eq!(out, StreamingOutput::default());
}

/// Gap 4: Default `check_stream_timeout` delegates to renderer when available.
#[test]
fn test_default_check_stream_timeout_delegates_to_renderer() {
    let plugin = DelegatingPlugin::new();
    // No text buffered, so timeout returns empty even when delegated.
    let out = plugin.check_stream_timeout();
    assert!(out.text_messages.is_empty());
    assert!(out.render_blocks.is_empty());
}

/// Gap 4: Default `check_stream_timeout` returns empty when no renderer.
#[test]
fn test_default_check_stream_timeout_returns_empty_when_no_renderer() {
    let plugin = NoRendererPlugin;
    let out = plugin.check_stream_timeout();
    assert_eq!(out, StreamingOutput::default());
}

/// Gap 4: Default `streaming_renderer()` returns `None`.
#[test]
fn test_default_streaming_renderer_returns_none() {
    let plugin = NoRendererPlugin;
    assert!(plugin.streaming_renderer().is_none());
}

/// Gap 4: Plugin with renderer returns `Some` from `streaming_renderer()`.
#[test]
fn test_streaming_renderer_returns_some_when_overridden() {
    let plugin = DelegatingPlugin::new();
    assert!(plugin.streaming_renderer().is_some());
}

// ===========================================================================
// Step 1.2: reply_ref & unavailable_media serde / round-trip tests
// ===========================================================================

/// Backward compat: JSON missing both new fields → defaults applied.
#[test]
fn test_reply_ref_unavailable_media_missing_fields_defaults() {
    let json = r#"{
        "platform": "feishu",
        "sender_id": "u1",
        "peer_id": "p1",
        "content": "hi",
        "timestamp": 1000
    }"#;
    let msg: NormalizedMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.reply_ref, None);
    assert!(msg.unavailable_media.is_empty());
}

/// Backward compat: JSON with null reply_ref → None.
#[test]
fn test_reply_ref_null_deserializes_to_none() {
    let json = r#"{
        "platform": "feishu",
        "sender_id": "u1",
        "peer_id": "p1",
        "content": "hi",
        "timestamp": 1000,
        "reply_ref": null
    }"#;
    let msg: NormalizedMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.reply_ref, None);
}

/// Backward compat: JSON with empty array unavailable_media → empty vec.
#[test]
fn test_unavailable_media_empty_array_deserializes_to_empty_vec() {
    let json = r#"{
        "platform": "feishu",
        "sender_id": "u1",
        "peer_id": "p1",
        "content": "hi",
        "timestamp": 1000,
        "unavailable_media": []
    }"#;
    let msg: NormalizedMessage = serde_json::from_str(json).unwrap();
    assert!(msg.unavailable_media.is_empty());
}

/// Round-trip: reply_ref set to Some, unavailable_media non-empty → preserved.
#[test]
fn test_reply_ref_unavailable_media_roundtrip_with_values() {
    let mut msg = make_normalized("t1");
    msg.reply_ref = Some("ref_abc".into());
    msg.unavailable_media = vec!["img_1".into(), "file_2".into()];

    let json = serde_json::to_string(&msg).unwrap();
    let de: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(de.reply_ref.as_deref(), Some("ref_abc"));
    assert_eq!(de.unavailable_media, vec!["img_1", "file_2"]);
}

/// Round-trip: reply_ref None, unavailable_media empty → preserved.
#[test]
fn test_reply_ref_unavailable_media_roundtrip_empty() {
    let msg = make_normalized("t2");
    assert_eq!(msg.reply_ref, None);
    assert!(msg.unavailable_media.is_empty());

    let json = serde_json::to_string(&msg).unwrap();
    let de: NormalizedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(de.reply_ref, None);
    assert!(de.unavailable_media.is_empty());
}

/// Default trait: reply_ref is None, unavailable_media is empty.
#[test]
fn test_normalized_default_reply_ref_and_unavailable_media() {
    let msg = NormalizedMessage::default();
    assert_eq!(msg.reply_ref, None);
    assert!(msg.unavailable_media.is_empty());
}

/// make_normalized helper: explicitly assigned defaults are correct.
#[test]
fn test_make_normalized_reply_ref_and_unavailable_media_defaults() {
    let msg = make_normalized("tenant_x");
    assert_eq!(msg.reply_ref, None);
    assert!(msg.unavailable_media.is_empty());
}
