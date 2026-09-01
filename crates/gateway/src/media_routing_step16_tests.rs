//! Step 1.6: boundary, failure, and model_supports_images tests.
//!
//! Extracted from `media_routing.rs` to stay under the 1000-line limit.

use super::*;
use closeclaw_common::im_plugin::{MediaType, MessageType};
use closeclaw_common::processor::ContentBlock;
use std::collections::HashMap;
use std::sync::Arc;

fn make_processed(
    content: &str,
    msg_type: MessageType,
    media_refs: Vec<closeclaw_common::im_plugin::MediaRef>,
) -> closeclaw_common::processor::ProcessedMessage {
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
    closeclaw_common::processor::ProcessedMessage {
        content_blocks: vec![closeclaw_common::processor::ContentBlock::Text(
            content.to_string(),
        )],
        metadata,
    }
}

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

// ── Boundary: exact threshold ──────────────────────────────────────────

/// Image size exactly equal to threshold → inline (≤ threshold).
#[tokio::test]
async fn exact_threshold_image_produces_image_block() {
    let tmp = tempfile::tempdir().unwrap();
    let data = [0x89, 0x50, 0x4E, 0x47]; // 4 bytes
    std::fs::write(tmp.path().join("eq.png"), &data).unwrap();
    let store = Arc::new(FsStore(tmp.path().to_path_buf()));
    let pm = make_processed(
        "",
        MessageType::Image,
        vec![closeclaw_common::im_plugin::MediaRef {
            key: "img_eq".into(),
            path: "eq.png".into(),
            media_type: MediaType::Image,
            size: 4,
            mime: "image/png".into(),
        }],
    );
    let blocks: Vec<ContentBlock> =
        build_context_content_blocks(&pm, Some(store.as_ref()), 4).await;
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        ContentBlock::Image { name, url } => {
            assert_eq!(name, "img_eq");
            assert!(url.starts_with("data:image/png;base64,"));
        }
        other => panic!("expected Image block for exact threshold, got {other:?}"),
    }
}

// ── Failure: image file missing ────────────────────────────────────────

/// Image file missing on disk → fallback to text reference token.
#[tokio::test]
async fn missing_image_file_falls_back_to_text_block() {
    let tmp = tempfile::tempdir().unwrap();
    // No file written — simulates read failure.
    let store = Arc::new(FsStore(tmp.path().to_path_buf()));
    let pm = make_processed(
        "",
        MessageType::Image,
        vec![closeclaw_common::im_plugin::MediaRef {
            key: "img_missing".into(),
            path: "nonexistent.png".into(),
            media_type: MediaType::Image,
            size: 100,
            mime: "image/png".into(),
        }],
    );
    let blocks: Vec<ContentBlock> =
        build_context_content_blocks(&pm, Some(store.as_ref()), 1024).await;
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0],
        ContentBlock::Text("[image: img_missing] nonexistent.png".into())
    );
}

// ── model_supports_images tests ────────────────────────────────────────

#[test]
fn model_supports_images_minimax_m3() {
    assert!(model_supports_images("minimax/MiniMax-M3"));
}

#[test]
fn model_supports_images_text_only() {
    // DeepSeek models are text-only
    assert!(!model_supports_images("deepseek/deepseek-v3-flash"));
}

#[test]
fn model_supports_images_unknown_model_fail_open() {
    // Model name without '/' → parse fails → fail-open (returns true)
    assert!(model_supports_images("no-slash-model"));
}

// ── Token format with empty path ───────────────────────────────────────

#[tokio::test]
async fn empty_path_produces_token_without_filename() {
    let pm = make_processed(
        "",
        MessageType::Image,
        vec![closeclaw_common::im_plugin::MediaRef {
            key: "k1".into(),
            path: "".into(),
            media_type: MediaType::Image,
            size: 100,
            mime: "image/png".into(),
        }],
    );
    let content = build_context_content(&pm, None, 0).await;
    assert_eq!(content, "[image: k1]");
}
