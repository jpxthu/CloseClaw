//! Media type routing and context content building for the Gateway.
//!
//! Extracted from `lib.rs` to keep the main file under the 1000-line limit.
//!
//! Handles:
//! - Message type–based validation (all types allowed; unavailable_media
//!   rejection for any type)
//! - Building context content strings with media reference tokens

use std::path::Path;

use super::Gateway;
use closeclaw_common::im_plugin::{MediaType, MessageType};
use closeclaw_common::processor::{ContentBlock, ProcessedMessage};
use closeclaw_common::MediaStoreAccess;

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

/// Build structured content blocks for an inbound message.
///
/// Returns a [`Vec<ContentBlock>`] where:
/// - Small images (≤ `image_threshold`) → [`ContentBlock::Image`] with
///   a `data:image/<mime>;base64,<data>` URL.
/// - Large images / non-image media → [`ContentBlock::Text`] reference
///   token (e.g. `[image: key] filename`).
/// - Text messages → single [`ContentBlock::Text`].
///
/// This is the structured-content counterpart to [`build_context_content`].
pub(crate) async fn build_context_content_blocks(
    processed: &ProcessedMessage,
    media_store: Option<&dyn MediaStoreAccess>,
    image_threshold: u64,
) -> Vec<ContentBlock> {
    let message_type = parse_message_type(processed);
    match message_type {
        MessageType::Text => {
            let text = processed.text_content().unwrap_or("").to_string();
            vec![ContentBlock::Text(text)]
        }
        MessageType::Post => {
            let text = processed.text_content().unwrap_or("").to_string();
            let full_refs = parse_full_media_refs(processed);
            if full_refs.is_empty() {
                return vec![ContentBlock::Text(text)];
            }
            let mut blocks = format_media_blocks(&full_refs, media_store, image_threshold).await;
            if !text.is_empty() {
                blocks.insert(0, ContentBlock::Text(text));
            }
            blocks
        }
        MessageType::Image | MessageType::File | MessageType::Audio => {
            let full_refs = parse_full_media_refs(processed);
            format_media_blocks(&full_refs, media_store, image_threshold).await
        }
    }
}

/// Format media references as structured [`ContentBlock`]s.
///
/// Small images (≤ threshold) produce [`ContentBlock::Image`] with a
/// data URI. All other types produce [`ContentBlock::Text`] reference
/// tokens.
async fn format_media_blocks(
    refs: &[FullMediaRefEntry],
    media_store: Option<&dyn MediaStoreAccess>,
    image_threshold: u64,
) -> Vec<ContentBlock> {
    let mut blocks = Vec::with_capacity(refs.len());
    for entry in refs {
        let filename_suffix = extract_filename(&entry.path)
            .map(|f| format!(" {f}"))
            .unwrap_or_default();
        if entry.media_type == MediaType::Image
            && entry.size > 0
            && (entry.size as u64) <= image_threshold
        {
            if let Some(data) = try_read_inline_image(media_store, &entry.path).await {
                let subtype = mime_to_data_uri_subtype(&entry.mime);
                let url = format!("data:image/{subtype};base64,{data}");
                blocks.push(ContentBlock::Image {
                    name: entry.key.clone(),
                    url,
                });
            } else {
                // Fallback: file not found → text reference token.
                blocks.push(ContentBlock::Text(format!(
                    "[image: {}]{filename_suffix}",
                    entry.key
                )));
            }
        } else {
            blocks.push(ContentBlock::Text(format!(
                "[{}: {}]{filename_suffix}",
                entry.media_type.label(),
                entry.key
            )));
        }
    }
    blocks
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
pub(crate) async fn build_context_content(
    processed: &ProcessedMessage,
    media_store: Option<&dyn MediaStoreAccess>,
    image_threshold: u64,
) -> String {
    let message_type = parse_message_type(processed);
    match message_type {
        MessageType::Text => processed.text_content().unwrap_or("").to_string(),
        MessageType::Post => {
            let text = processed.text_content().unwrap_or("").to_string();
            let full_refs = parse_full_media_refs(processed);
            if full_refs.is_empty() {
                return text;
            }
            let media_part =
                format_media_tokens_with_inline(&full_refs, media_store, image_threshold).await;
            if text.is_empty() {
                media_part
            } else {
                format!("{text} {media_part}")
            }
        }
        MessageType::Image | MessageType::File | MessageType::Audio => {
            let full_refs = parse_full_media_refs(processed);
            format_media_tokens_with_inline(&full_refs, media_store, image_threshold).await
        }
    }
}

/// A full media reference parsed from metadata, including path and size
/// for threshold-based inline decisions.
struct FullMediaRefEntry {
    media_type: MediaType,
    key: String,
    path: String,
    size: i64,
    mime: String,
}

/// Parse full media references from the same metadata source.
fn parse_full_media_refs(processed: &ProcessedMessage) -> Vec<FullMediaRefEntry> {
    let Some(refs_json) = processed.metadata.get("media_refs") else {
        return Vec::new();
    };
    let refs: Vec<closeclaw_common::im_plugin::MediaRef> =
        serde_json::from_str(refs_json).unwrap_or_default();
    refs.into_iter()
        .map(|r| FullMediaRefEntry {
            media_type: r.media_type,
            key: r.key,
            path: r.path,
            size: r.size,
            mime: r.mime,
        })
        .collect()
}

/// Extract the filename component from a path string.
///
/// Returns `None` when the path is empty or has no usable filename
/// component (e.g. ends with `..` or a separator).
fn extract_filename(path: &str) -> Option<&str> {
    if path.is_empty() {
        return None;
    }
    Path::new(path).file_name().and_then(|n| n.to_str())
}

/// Format media tokens with inline base64 for small images.
///
/// Token format: `[image: key] filename.ext` (when a filename is
/// available) or `[image: key]` (when the path is empty or has no
/// extractable filename). The filename annotation helps the LLM
/// identify the media without exposing local filesystem paths.
///
/// For Image-type refs whose file size is ≤ `image_threshold`, the
/// file content is read and base64-encoded inline after the token.
/// Other types keep the standard `[type: key]` reference token with
/// the filename annotation.
async fn format_media_tokens_with_inline(
    refs: &[FullMediaRefEntry],
    media_store: Option<&dyn MediaStoreAccess>,
    image_threshold: u64,
) -> String {
    let mut parts = Vec::with_capacity(refs.len());
    for entry in refs {
        let filename_suffix = extract_filename(&entry.path)
            .map(|f| format!(" {f}"))
            .unwrap_or_default();
        if entry.media_type == MediaType::Image
            && entry.size > 0
            && (entry.size as u64) <= image_threshold
        {
            if let Some(data) = try_read_inline_image(media_store, &entry.path).await {
                parts.push(format!("[image: {}]{filename_suffix}\n{data}", entry.key));
            } else {
                parts.push(format!("[image: {}]{filename_suffix}", entry.key));
            }
        } else {
            parts.push(format!(
                "[{}: {}]{filename_suffix}",
                entry.media_type.label(),
                entry.key
            ));
        }
    }
    parts.join(" ")
}

/// Extract the MIME subtype for use in data URIs.
///
/// Given `"image/png"`, returns `"png"`. For unknown formats,
/// falls back to `"png"`.
fn mime_to_data_uri_subtype(mime: &str) -> &str {
    mime.strip_prefix("image/").unwrap_or("png")
}

/// Attempt to read a media file and return its base64-encoded content.
///
/// Returns `None` if the media store is not available, the file cannot
/// be resolved, or reading fails.
async fn try_read_inline_image(
    media_store: Option<&dyn MediaStoreAccess>,
    path: &str,
) -> Option<String> {
    let store = media_store?;
    let temp_ref = closeclaw_common::MediaRef {
        key: String::new(),
        path: path.to_string(),
        media_type: MediaType::Image,
        size: 0,
        mime: String::new(),
    };
    let abs_path = store.resolve_ref(&temp_ref).ok()?;
    let bytes = tokio::fs::read(&abs_path).await.ok()?;
    Some(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &bytes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

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

    #[tokio::test]
    async fn text_message_returns_original_content() {
        let pm = make_processed("hello world", MessageType::Text, vec![]);
        assert_eq!(build_context_content(&pm, None, 0).await, "hello world");
    }

    #[tokio::test]
    async fn text_message_empty_content() {
        let pm = make_processed("", MessageType::Text, vec![]);
        assert_eq!(build_context_content(&pm, None, 0).await, "");
    }

    #[tokio::test]
    async fn image_message_with_refs() {
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_abc123".into(),
                path: "/tmp/img.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 1024,
                mime: "image/png".into(),
            }],
        );
        assert_eq!(
            build_context_content(&pm, None, 0).await,
            "[image: img_abc123] img.png"
        );
    }

    #[tokio::test]
    async fn file_message_with_refs() {
        let pm = make_processed(
            "",
            MessageType::File,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "doc_xyz".into(),
                path: "/tmp/doc.pdf".into(),
                media_type: closeclaw_common::im_plugin::MediaType::File,
                size: 2048,
                mime: "application/pdf".into(),
            }],
        );
        assert_eq!(
            build_context_content(&pm, None, 0).await,
            "[file: doc_xyz] doc.pdf"
        );
    }

    #[tokio::test]
    async fn audio_message_with_refs() {
        let pm = make_processed(
            "",
            MessageType::Audio,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "voice_001".into(),
                path: "/tmp/voice.ogg".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Audio,
                size: 512,
                mime: "audio/ogg".into(),
            }],
        );
        assert_eq!(
            build_context_content(&pm, None, 0).await,
            "[audio: voice_001] voice.ogg"
        );
    }

    #[tokio::test]
    async fn image_message_without_refs_returns_empty() {
        let pm = make_processed("", MessageType::Image, vec![]);
        assert_eq!(build_context_content(&pm, None, 0).await, "");
    }

    #[tokio::test]
    async fn post_with_text_and_media_refs() {
        let pm = make_processed(
            "check this image",
            MessageType::Post,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "pic_42".into(),
                path: "/tmp/pic.jpg".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 512,
                mime: "image/jpeg".into(),
            }],
        );
        assert_eq!(
            build_context_content(&pm, None, 0).await,
            "check this image [image: pic_42] pic.jpg"
        );
    }

    #[tokio::test]
    async fn post_with_media_only_no_text() {
        let pm = make_processed(
            "",
            MessageType::Post,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "vid_7".into(),
                path: "/tmp/vid.mp4".into(),
                media_type: closeclaw_common::im_plugin::MediaType::File,
                size: 4096,
                mime: "video/mp4".into(),
            }],
        );
        assert_eq!(
            build_context_content(&pm, None, 0).await,
            "[file: vid_7] vid.mp4"
        );
    }

    #[tokio::test]
    async fn post_with_no_media_returns_text() {
        let pm = make_processed("just text", MessageType::Post, vec![]);
        assert_eq!(build_context_content(&pm, None, 0).await, "just text");
    }

    #[tokio::test]
    async fn multiple_media_refs() {
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
        assert_eq!(
            build_context_content(&pm, None, 0).await,
            "[image: a1] [file: f1]"
        );
    }

    #[tokio::test]
    async fn reference_token_does_not_contain_local_path() {
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
        let content = build_context_content(&pm, None, 0).await;
        assert!(
            !content.contains("/home"),
            "context content must not contain local paths: {content}"
        );
        assert!(
            !content.contains("secret/"),
            "context content must not contain path components: {content}"
        );
        assert!(content.contains("secret_key"));
        // Filename annotation is allowed — it is not a filesystem path
        assert!(content.contains("photo.jpg"));
    }

    // ── build_context_content_blocks tests ─────────────────────────────

    /// Small image (≤ threshold) → ContentBlock::Image with data URI.
    #[tokio::test]
    async fn blocks_small_image_produces_image_block() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("small.png"), &[0x89, 0x50, 0x4E, 0x47]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_small".into(),
                path: "small.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 4,
                mime: "image/png".into(),
            }],
        );
        let blocks = build_context_content_blocks(&pm, Some(store.as_ref()), 1024).await;
        assert_eq!(blocks.len(), 1, "should produce one block: {blocks:?}");
        match &blocks[0] {
            ContentBlock::Image { name, url } => {
                assert_eq!(name, "img_small");
                assert!(
                    url.starts_with("data:image/png;base64,"),
                    "should be a data URI: {url}"
                );
            }
            other => panic!("expected Image block, got {other:?}"),
        }
    }

    /// Large image (> threshold) → ContentBlock::Text reference token.
    #[tokio::test]
    async fn blocks_large_image_produces_text_block() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("large.png"), &[0u8; 2048]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_large".into(),
                path: "large.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 2048,
                mime: "image/png".into(),
            }],
        );
        let blocks = build_context_content_blocks(&pm, Some(store.as_ref()), 1024).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0],
            ContentBlock::Text("[image: img_large] large.png".into())
        );
    }

    /// File type → always ContentBlock::Text reference token.
    #[tokio::test]
    async fn blocks_file_type_produces_text_block() {
        let pm = make_processed(
            "",
            MessageType::File,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "doc1".into(),
                path: "doc.pdf".into(),
                media_type: closeclaw_common::im_plugin::MediaType::File,
                size: 100,
                mime: "application/pdf".into(),
            }],
        );
        let blocks = build_context_content_blocks(&pm, None, 1024).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], ContentBlock::Text("[file: doc1] doc.pdf".into()));
    }

    /// Text message → ContentBlock::Text.
    #[tokio::test]
    async fn blocks_text_message_produces_text_block() {
        let pm = make_processed("hello world", MessageType::Text, vec![]);
        let blocks = build_context_content_blocks(&pm, None, 0).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], ContentBlock::Text("hello world".into()));
    }

    /// Mixed: text + small image + file → multiple blocks.
    #[tokio::test]
    async fn blocks_mixed_content_produces_multiple_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("pic.jpg"), &[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "check this",
            MessageType::Post,
            vec![
                closeclaw_common::im_plugin::MediaRef {
                    key: "s".into(),
                    path: "pic.jpg".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::Image,
                    size: 4,
                    mime: "image/jpeg".into(),
                },
                closeclaw_common::im_plugin::MediaRef {
                    key: "f".into(),
                    path: "doc.pdf".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::File,
                    size: 100,
                    mime: "application/pdf".into(),
                },
            ],
        );
        let blocks = build_context_content_blocks(&pm, Some(store.as_ref()), 1024).await;
        assert_eq!(blocks.len(), 3, "text + image + file: {blocks:?}");
        // First block: text
        assert_eq!(blocks[0], ContentBlock::Text("check this".into()));
        // Second block: image
        match &blocks[1] {
            ContentBlock::Image { name, url } => {
                assert_eq!(name, "s");
                assert!(url.starts_with("data:image/jpeg;base64,"));
            }
            other => panic!("expected Image block, got {other:?}"),
        }
        // Third block: file reference
        assert_eq!(blocks[2], ContentBlock::Text("[file: f] doc.pdf".into()));
    }

    /// No media store → fallback to text reference tokens.
    #[tokio::test]
    async fn blocks_no_store_falls_back_to_text() {
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_no_store".into(),
                path: "inbound/img.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 100,
                mime: "image/png".into(),
            }],
        );
        let blocks = build_context_content_blocks(&pm, None, 1024).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0],
            ContentBlock::Text("[image: img_no_store] img.png".into())
        );
    }

    /// Empty path → token without filename annotation.
    #[tokio::test]
    async fn blocks_empty_path_produces_token_without_filename() {
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "k1".into(),
                path: "".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 100,
                mime: "image/png".into(),
            }],
        );
        let blocks = build_context_content_blocks(&pm, None, 0).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], ContentBlock::Text("[image: k1]".into()));
    }

    // ── Threshold-based inline image tests ──────────────────────────────

    /// Simple file-system-backed store for testing.
    struct FsStore(std::path::PathBuf);

    impl closeclaw_common::MediaStoreAccess for FsStore {
        fn resolve_ref(
            &self,
            media_ref: &closeclaw_common::MediaRef,
        ) -> Result<std::path::PathBuf, closeclaw_common::MediaStoreError> {
            if media_ref.path.is_empty() {
                return Err(closeclaw_common::MediaStoreError::NoPath);
            }
            let full = self.0.join(&media_ref.path);
            if !full.exists() {
                return Err(closeclaw_common::MediaStoreError::FileNotFound(full));
            }
            Ok(full)
        }
    }

    /// Small image (≤ threshold) → base64 inline data appears in context.
    #[tokio::test]
    async fn small_image_below_threshold_inlines_base64() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("small.png"), &[0x89, 0x50, 0x4E, 0x47]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_small".into(),
                path: "small.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 4,
                mime: "image/png".into(),
            }],
        );
        let content = build_context_content(&pm, Some(store.as_ref()), 1024).await;
        assert!(
            content.starts_with("[image: img_small] small.png"),
            "token should include filename: {content}"
        );
        assert!(
            content.contains("iVBOR"),
            "should contain base64 of the PNG data"
        );
    }

    /// Large image (> threshold) → reference token only, no base64.
    #[tokio::test]
    async fn large_image_above_threshold_keeps_reference() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("large.png"), &[0u8; 2048]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_large".into(),
                path: "large.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 2048,
                mime: "image/png".into(),
            }],
        );
        let content = build_context_content(&pm, Some(store.as_ref()), 1024).await;
        assert_eq!(content, "[image: img_large] large.png");
    }

    /// File type → always reference token, never inline.
    #[tokio::test]
    async fn file_type_always_uses_reference_token() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("doc.pdf"), &[0u8; 100]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "",
            MessageType::File,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "doc1".into(),
                path: "doc.pdf".into(),
                media_type: closeclaw_common::im_plugin::MediaType::File,
                size: 100,
                mime: "application/pdf".into(),
            }],
        );
        let content = build_context_content(&pm, Some(store.as_ref()), 1024).await;
        assert_eq!(content, "[file: doc1] doc.pdf");
    }

    /// Audio type → always reference token, never inline.
    #[tokio::test]
    async fn audio_type_always_uses_reference_token() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("voice.ogg"), &[0u8; 50]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "",
            MessageType::Audio,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "aud1".into(),
                path: "voice.ogg".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Audio,
                size: 50,
                mime: "audio/ogg".into(),
            }],
        );
        let content = build_context_content(&pm, Some(store.as_ref()), 1024).await;
        assert_eq!(content, "[audio: aud1] voice.ogg");
    }

    /// No media store → reference token only (graceful fallback).
    #[tokio::test]
    async fn no_media_store_falls_back_to_reference_token() {
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_no_store".into(),
                path: "inbound/img.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 100,
                mime: "image/png".into(),
            }],
        );
        let content = build_context_content(&pm, None, 1024).await;
        assert_eq!(content, "[image: img_no_store] img.png");
    }

    /// File not found on disk → reference token (graceful fallback).
    #[tokio::test]
    async fn missing_file_falls_back_to_reference_token() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "img_missing".into(),
                path: "missing.png".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 100,
                mime: "image/png".into(),
            }],
        );
        let content = build_context_content(&pm, Some(store.as_ref()), 1024).await;
        assert_eq!(content, "[image: img_missing] missing.png");
    }

    /// Post with text + small image → text + inline base64.
    #[tokio::test]
    async fn post_text_and_small_image_inlines_base64() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("photo.jpg"), &[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "look at this",
            MessageType::Post,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "pic".into(),
                path: "photo.jpg".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 4,
                mime: "image/jpeg".into(),
            }],
        );
        let content = build_context_content(&pm, Some(store.as_ref()), 1024).await;
        assert!(content.starts_with("look at this [image: pic] photo.jpg"));
        assert!(content.contains("/9j/"), "should contain base64 JPEG data");
    }

    /// Mixed refs: small image + large file → inline image + reference file.
    #[tokio::test]
    async fn mixed_refs_inline_small_image_keep_file_reference() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("small.png"), &[0x89, 0x50, 0x4E, 0x47]).unwrap();
        std::fs::write(tmp.path().join("big.pdf"), &[0u8; 5000]).unwrap();
        let store = Arc::new(FsStore(tmp.path().to_path_buf()));

        let pm = make_processed(
            "",
            MessageType::Post,
            vec![
                closeclaw_common::im_plugin::MediaRef {
                    key: "s".into(),
                    path: "small.png".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::Image,
                    size: 4,
                    mime: "image/png".into(),
                },
                closeclaw_common::im_plugin::MediaRef {
                    key: "f".into(),
                    path: "big.pdf".into(),
                    media_type: closeclaw_common::im_plugin::MediaType::File,
                    size: 5000,
                    mime: "application/pdf".into(),
                },
            ],
        );
        let content = build_context_content(&pm, Some(store.as_ref()), 1024).await;
        assert!(
            content.contains("[image: s] small.png"),
            "small image token should include filename"
        );
        assert!(content.contains("iVBOR"), "small image should be inlined");
        assert!(
            content.contains("[file: f] big.pdf"),
            "file should remain a reference with filename"
        );
    }

    // ── extract_filename unit tests ──────────────────────────────────

    #[test]
    fn extract_filename_with_extension() {
        assert_eq!(extract_filename("small.png"), Some("small.png"));
        assert_eq!(extract_filename("doc.pdf"), Some("doc.pdf"));
        assert_eq!(extract_filename("/tmp/my_file.jpg"), Some("my_file.jpg"));
    }

    #[test]
    fn extract_filename_without_extension() {
        assert_eq!(extract_filename("/tmp/img"), Some("img"));
        assert_eq!(extract_filename("/tmp/voice"), Some("voice"));
    }

    #[test]
    fn extract_filename_empty_path() {
        assert_eq!(extract_filename(""), None);
    }

    #[test]
    fn extract_filename_deep_path() {
        assert_eq!(
            extract_filename("inbound/img_abc_photo.png"),
            Some("img_abc_photo.png")
        );
    }

    // ── Token format with empty path (no filename annotation) ────────

    #[tokio::test]
    async fn empty_path_produces_token_without_filename() {
        let pm = make_processed(
            "",
            MessageType::Image,
            vec![closeclaw_common::im_plugin::MediaRef {
                key: "k1".into(),
                path: "".into(),
                media_type: closeclaw_common::im_plugin::MediaType::Image,
                size: 100,
                mime: "image/png".into(),
            }],
        );
        let content = build_context_content(&pm, None, 0).await;
        assert_eq!(content, "[image: k1]");
    }
}
