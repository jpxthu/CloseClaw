//! Media type routing and context content building for the Gateway.
//!
//! Extracted from `lib.rs` to keep the main file under the 1000-line limit.
//!
//! Handles:
//! - Message type–based validation (all types allowed; unavailable_media
//!   rejection for any type)
//! - Building context content strings with media reference tokens

use super::Gateway;
use closeclaw_common::im_plugin::MessageType;
use closeclaw_common::processor::ProcessedMessage;

use crate::HandleResult;

/// Result of inbound pre-validation gates.
pub(crate) enum InboundValidation {
    /// Message passed all checks — continue processing.
    Continue,
    /// Message rejected; caller must return this result immediately.
    Reject(HandleResult),
    /// Message rejected silently (no user reply); caller must return `None`.
    RejectSilently,
}

/// Parse `message_type` from processed message metadata.
///
/// Returns the deserialized [`MessageType`], defaulting to
/// [`MessageType::Text`] when the key is absent or unparseable.
/// Logs a warning when deserialization fails (invalid JSON or
/// unrecognized enum variant).
pub(crate) fn parse_message_type(processed: &ProcessedMessage) -> MessageType {
    let Some(raw) = processed.metadata.get("message_type") else {
        return MessageType::default();
    };
    match serde_json::from_str::<MessageType>(raw) {
        Ok(mt) => mt,
        Err(e) => {
            tracing::warn!(
                raw_type = %raw,
                error = %e,
                "failed to deserialize message_type from metadata, defaulting to Text"
            );
            MessageType::default()
        }
    }
}

/// Validate an inbound message against media availability and size limits.
///
/// Unlike the previous implementation, all message types (text, post,
/// image, file, audio) are accepted. Rejection occurs only when:
/// - `unavailable_media` is non-empty → "该消息内容无法获取"
/// - Text content exceeds `max_message_size` → "消息过长，请缩短后重试"
pub(crate) async fn validate_inbound(
    gw: &Gateway,
    processed: &ProcessedMessage,
    peer_id: &str,
    channel: &str,
) -> InboundValidation {
    // unavailable_media check — applies to ALL message types.
    let unavailable_media: Vec<String> = processed
        .metadata
        .get("unavailable_media")
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    if !unavailable_media.is_empty() {
        tracing::info!(
            unavailable_count = unavailable_media.len(),
            "rejecting message with unavailable media"
        );
        return match gw
            .reject_with_reply(peer_id, channel, "该消息内容无法获取")
            .await
        {
            Some(r) => InboundValidation::Reject(r),
            None => InboundValidation::RejectSilently,
        };
    }
    // max_message_size — only meaningful when text content exists.
    let content = processed.text_content().unwrap_or("").to_string();
    if content.len() > gw.config.max_message_size {
        tracing::warn!(
            peer_id = %peer_id,
            size = content.len(),
            limit = gw.config.max_message_size,
            "inbound message exceeds max_message_size"
        );
        return match gw
            .reject_with_reply(peer_id, channel, "消息过长，请缩短后重试")
            .await
        {
            Some(r) => InboundValidation::Reject(r),
            None => InboundValidation::RejectSilently,
        };
    }
    InboundValidation::Continue
}

/// Build the context content string for an inbound message.
///
/// For text messages, returns the original text content unchanged.
/// For media messages (image/file/audio), generates reference tokens
/// (`[image: key]`, `[file: key]`, `[audio: key]`) from `media_refs`
/// in metadata. No local file system paths appear in the output.
///
/// The content string is consumed by `route_and_dispatch` for slash
/// command detection and session/LLM routing.
pub(crate) fn build_context_content(processed: &ProcessedMessage) -> String {
    let message_type = parse_message_type(processed);
    match message_type {
        MessageType::Text => processed.text_content().unwrap_or("").to_string(),
        MessageType::Post => {
            let text = processed.text_content().unwrap_or("").to_string();
            let media_refs = parse_media_refs(processed);
            if media_refs.is_empty() {
                return text;
            }
            let media_part = format_media_tokens(&media_refs);
            if text.is_empty() {
                media_part
            } else {
                format!("{text} {media_part}")
            }
        }
        MessageType::Image | MessageType::File | MessageType::Audio => {
            let media_refs = parse_media_refs(processed);
            format_media_tokens(&media_refs)
        }
    }
}

/// A media reference parsed from metadata, pairing a [`MediaType`] with
/// its resource key.
struct MediaRefEntry {
    media_type: closeclaw_common::im_plugin::MediaType,
    key: String,
}

/// Parse media references from metadata into typed entries.
///
/// Uses the `media_refs` JSON array in metadata. Only the `key` field
/// is used; `path` is intentionally omitted (design doc: no local
/// paths in context content).
fn parse_media_refs(processed: &ProcessedMessage) -> Vec<MediaRefEntry> {
    let Some(refs_json) = processed.metadata.get("media_refs") else {
        return Vec::new();
    };
    let refs: Vec<closeclaw_common::im_plugin::MediaRef> =
        serde_json::from_str(refs_json).unwrap_or_default();
    refs.into_iter()
        .map(|r| MediaRefEntry {
            media_type: r.media_type,
            key: r.key,
        })
        .collect()
}

/// Format media reference entries into `[type: key]` token strings,
/// joined by spaces.
fn format_media_tokens(refs: &[MediaRefEntry]) -> String {
    refs.iter()
        .map(|entry| format!("[{}: {}]", entry.media_type.label(), entry.key))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_processed(
        content: &str,
        msg_type: MessageType,
        media_refs: Vec<closeclaw_common::im_plugin::MediaRef>,
    ) -> ProcessedMessage {
        let mut metadata = HashMap::new();
        metadata.insert(
            "message_type".to_string(),
            serde_json::to_string(&msg_type).unwrap(),
        );
        if !media_refs.is_empty() {
            metadata.insert(
                "media_refs".to_string(),
                serde_json::to_string(&media_refs).unwrap(),
            );
        }
        ProcessedMessage {
            content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                content.to_string(),
            )],
            metadata,
        }
    }

    #[test]
    fn text_message_returns_original_content() {
        let pm = make_processed("hello world", MessageType::Text, vec![]);
        assert_eq!(build_context_content(&pm), "hello world");
    }

    #[test]
    fn text_message_empty_content() {
        let pm = make_processed("", MessageType::Text, vec![]);
        assert_eq!(build_context_content(&pm), "");
    }

    #[test]
    fn image_message_with_refs() {
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_abc123".into(),
                path: "/tmp/img".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 1024,
                mime: "image/png".into(),
            }],
        );
        assert_eq!(build_context_content(&pm), "[image: img_abc123]");
    }

    #[test]
    fn file_message_with_refs() {
        let pm = make_processed(
            "",
            MessageType::File,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "doc_xyz".into(),
                path: "/tmp/doc".into(),
                media_type: closeclaw_common::im_plugin::MediaType::File,
                size: 2048,
                mime: "application/pdf".into(),
            }],
        );
        assert_eq!(build_context_content(&pm), "[file: doc_xyz]");
    }

    #[test]
    fn audio_message_with_refs() {
        let pm = make_processed(
            "",
            MessageType::Audio,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "voice_001".into(),
                path: "/tmp/voice".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Audio,
                size: 512,
                mime: "audio/ogg".into(),
            }],
        );
        assert_eq!(build_context_content(&pm), "[audio: voice_001]");
    }

    #[test]
    fn image_message_without_refs_returns_empty() {
        let pm = make_processed("", MessageType::Image, vec![]);
        assert_eq!(build_context_content(&pm), "");
    }

    #[test]
    fn post_with_text_and_media_refs() {
        let pm = make_processed(
            "check this image",
            MessageType::Post,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "pic_42".into(),
                path: "/tmp/pic".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 512,
                mime: "image/jpeg".into(),
            }],
        );
        assert_eq!(
            build_context_content(&pm),
            "check this image [image: pic_42]"
        );
    }

    #[test]
    fn post_with_media_only_no_text() {
        let pm = make_processed(
            "",
            MessageType::Post,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "vid_7".into(),
                path: "/tmp/vid".into(),
                media_type: closeclaw_common::im_plugin::MediaType::File,
                size: 4096,
                mime: "video/mp4".into(),
            }],
        );
        assert_eq!(build_context_content(&pm), "[file: vid_7]");
    }

    #[test]
    fn post_with_no_media_returns_text() {
        let pm = make_processed("just text", MessageType::Post, vec![]);
        assert_eq!(build_context_content(&pm), "just text");
    }

    #[test]
    fn multiple_media_refs() {
        let pm = make_processed(
            "",
            MessageType::Post,
            vec![
                closeclaw_common::im_plugin::MediaRef {
                    key: "a1".into(),
                    path: "".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::Image,
                    size: 100,
                    mime: "image/png".into(),
                },
                closeclaw_common::im_plugin::MediaRef {
                    key: "f1".into(),
                    path: "".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::File,
                    size: 200,
                    mime: "text/plain".into(),
                },
            ],
        );
        assert_eq!(build_context_content(&pm), "[image: a1] [file: f1]");
    }

    #[test]
    fn reference_token_does_not_contain_local_path() {
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "secret_key".into(),
                path: "/home/user/secret/photo.jpg".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 1024,
                mime: "image/jpeg".into(),
            }],
        );
        let content = build_context_content(&pm);
        assert!(
            !content.contains("/home"),
            "context content must not contain local paths: {content}"
        );
        assert!(
            !content.contains("secret/photo.jpg"),
            "context content must not contain path components: {content}"
        );
        assert!(content.contains("secret_key"));
    }
}
