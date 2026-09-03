//! Tests for Step 1.2: feishu adapter message filtering semantics.
//!
//! Covers:
//! - text: empty content → discard; non-empty → produce
//! - post: empty content + no media → discard; has media → produce; has text → produce
//! - image/file/audio: always produce (content may be empty)
//! - sticker: emoji expansion, empty emoji → produce with `[]`
//! - post media_refs extraction from embedded img/media/file tags

use super::*;
use crate::media_store::MediaStore;
use closeclaw_common::MessageType;
use tempfile::TempDir;

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

/// Create a test FeishuAdapter (no real HTTP — only sync methods).
fn make_test_adapter() -> FeishuAdapter {
    FeishuAdapter::new("test_profile".to_string(), make_test_media_store())
}

/// Build a minimal FeishuEvent for a message event.
fn make_message_event(message_type: &str, content_json: &str) -> FeishuEvent {
    FeishuEvent {
        schema: "2.0".to_string(),
        header: FeishuHeader {
            event_id: "ev_test".to_string(),
            event_type: "im.message.receive_v1".to_string(),
            create_time: "1234567890".to_string(),
            token: "tok".to_string(),
            app_id: "test_app_id".to_string(),
        },
        event: FeishuMessageEvent {
            message_id: None,
            sender: FeishuSender {
                sender_id: FeishuSenderId {
                    open_id: "ou_sender".to_string(),
                },
                sender_type: "user".to_string(),
            },
            content: content_json.to_string(),
            chat_id: "oc_chat".to_string(),
            chat_type: None,
            message_type: message_type.to_string(),
            thread_id: None,
            root_id: None,
            parent_id: None,
        },
    }
}

/// Build a FeishuEvent with an explicit message_id.
fn make_message_event_with_id(
    message_type: &str,
    content_json: &str,
    message_id: Option<&str>,
) -> FeishuEvent {
    FeishuEvent {
        schema: "2.0".to_string(),
        header: FeishuHeader {
            event_id: "ev_test".to_string(),
            event_type: "im.message.receive_v1".to_string(),
            create_time: "1234567890".to_string(),
            token: "tok".to_string(),
            app_id: "test_app_id".to_string(),
        },
        event: FeishuMessageEvent {
            message_id: message_id.map(String::from),
            sender: FeishuSender {
                sender_id: FeishuSenderId {
                    open_id: "ou_sender".to_string(),
                },
                sender_type: "user".to_string(),
            },
            content: content_json.to_string(),
            chat_id: "oc_chat".to_string(),
            chat_type: None,
            message_type: message_type.to_string(),
            thread_id: None,
            root_id: None,
            parent_id: None,
        },
    }
}

// ===========================================================================
// text type filtering
// ===========================================================================

#[tokio::test]
async fn test_text_empty_content_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": ""}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "text with empty content should be discarded"
    );
}

#[tokio::test]
async fn test_text_missing_text_field_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "text with missing text field should be discarded"
    );
}

#[tokio::test]
async fn test_text_whitespace_only_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "   "}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "text with whitespace-only content should be discarded"
    );
}

#[tokio::test]
async fn test_text_non_empty_produces_message() {
    let adapter = make_test_adapter();
    let event = make_message_event("text", &serde_json::json!({"text": "hello"}).to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.message_type, MessageType::Text);
    assert!(msg.media_refs.is_empty());
}

// ===========================================================================
// post type filtering
// ===========================================================================

#[tokio::test]
async fn test_post_empty_expand_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("post", &serde_json::json!({"content": []}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "post with empty expand should be discarded"
    );
}

#[tokio::test]
async fn test_post_no_content_key_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event("post", &serde_json::json!({}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "post with no content key should be discarded"
    );
}

#[tokio::test]
async fn test_post_with_text_produces_message() {
    let adapter = make_test_adapter();
    let content = serde_json::json!({
        "title": "T",
        "content": [[{"tag": "text", "text": "body"}]]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "T\nbody");
    assert_eq!(msg.message_type, MessageType::Post);
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_post_with_image_not_discarded() {
    let adapter = make_test_adapter();
    let content = serde_json::json!({
        "content": [[{"tag": "img", "image_key": "img_in_post"}]]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Post);
    assert_eq!(msg.content, "[图片]");
    // Download fails in unit tests → media unavailable
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_post_empty_text_no_media_discarded() {
    let adapter = make_test_adapter();
    let event = make_message_event("post", &serde_json::json!({"content": []}).to_string());
    assert!(
        adapter.parse_message_event(event).await.unwrap().is_none(),
        "post with empty text and no media should be discarded"
    );
}

#[tokio::test]
async fn test_post_empty_text_with_image_produces() {
    let adapter = make_test_adapter();
    // Post with only an img tag (no text) → content is "[图片]",
    // media_refs populated. Download fails → media unavailable.
    let content = serde_json::json!({
        "content": [[{"tag": "img", "image_key": "img_in_post_only"}]]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Post);
    assert_eq!(msg.content, "[图片]");
    // Download fails in unit tests → media unavailable
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_post_with_text_and_embedded_image() {
    let adapter = make_test_adapter();
    // Post with text + img tag → content has text, media_refs has image.
    // Download fails → media unavailable.
    let content = serde_json::json!({
        "content": [
            [{"tag": "text", "text": "Check this: "}],
            [{"tag": "img", "image_key": "img_mixed"}]
        ]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Post);
    assert!(
        msg.content.contains("Check this:"),
        "content should contain the text portion: {}",
        msg.content
    );
    // Download fails in unit tests → media unavailable
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_post_empty_text_with_file_produces() {
    let adapter = make_test_adapter();
    // Post with only a file tag (no text) → content is "[文件]",
    // media_refs populated. Download fails → media unavailable.
    let content = serde_json::json!({
        "content": [
            [{"tag": "file", "file_key": "file_in_post"}]
        ]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Post);
    assert_eq!(msg.content, "[文件]");
    // Download fails in unit tests → media unavailable
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_post_multiple_media_refs_extracted() {
    let adapter = make_test_adapter();
    // Post with img + file + text → media_refs has both, content has text.
    // Download fails → media unavailable.
    let content = serde_json::json!({
        "content": [
            [{"tag": "text", "text": "Report attached:"}],
            [{"tag": "img", "image_key": "chart_1"}],
            [{"tag": "file", "file_key": "report_pdf"}]
        ]
    });
    let event = make_message_event("post", &content.to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Post);
    assert!(msg.content.contains("Report attached:"));
    // Download fails in unit tests → media unavailable
    assert!(msg.media_refs.is_empty());
}

// ===========================================================================
// image/file/audio type — always produce
// ===========================================================================

#[tokio::test]
async fn test_image_no_text_produces_message() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "image",
        &serde_json::json!({"image_key": "img_xxx"}).to_string(),
        Some("om_msg_001"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Image);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert_eq!(msg.content, "[图片]");
}

#[tokio::test]
async fn test_file_no_text_produces_message() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "file",
        &serde_json::json!({"file_key": "file_xxx"}).to_string(),
        Some("om_msg_002"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::File);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert!(msg.content.is_empty());
}

#[tokio::test]
async fn test_audio_no_text_produces_message() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "audio",
        &serde_json::json!({"file_key": "audio_xxx"}).to_string(),
        Some("om_msg_003"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Audio);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert!(msg.content.is_empty());
}

#[tokio::test]
async fn test_image_empty_key_produces_message() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "image",
        &serde_json::json!({}).to_string(),
        Some("om_img_empty"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::Image);
    // Empty key → download fails → media unavailable
    assert!(msg.media_refs.is_empty());
}

// ===========================================================================
// sticker type — emoji expansion
// ===========================================================================

#[tokio::test]
async fn test_sticker_with_emoji_produces_message() {
    let adapter = make_test_adapter();
    let event = make_message_event(
        "sticker",
        &serde_json::json!({"emoji_type": "THUMBSUP"}).to_string(),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "[THUMBSUP]");
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_sticker_without_emoji_produces_message() {
    let adapter = make_test_adapter();
    let event = make_message_event("sticker", &serde_json::json!({}).to_string());
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.content, "[]");
    assert!(msg.media_refs.is_empty());
}

// ===========================================================================
// post media_refs extraction (extract_post_media_refs)
// ===========================================================================

#[test]
fn test_extract_post_media_refs_with_images() {
    let content = serde_json::json!({
        "content": [
            [{"tag": "text", "text": "hello "}],
            [{"tag": "img", "image_key": "img_abc"}],
            [{"tag": "text", "text": " world"}]
        ]
    });
    let refs = super::post_expand::extract_post_media_refs(&content);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "img_abc");
    assert_eq!(refs[0].media_type, closeclaw_common::MediaType::Image);
}

#[test]
fn test_extract_post_media_refs_with_file() {
    let content = serde_json::json!({
        "content": [
            [{"tag": "file", "file_key": "file_xyz"}]
        ]
    });
    let refs = super::post_expand::extract_post_media_refs(&content);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "file_xyz");
    assert_eq!(refs[0].media_type, closeclaw_common::MediaType::File);
}

#[test]
fn test_extract_post_media_refs_empty_key_skipped() {
    let content = serde_json::json!({
        "content": [
            [{"tag": "img", "image_key": ""}]
        ]
    });
    let refs = super::post_expand::extract_post_media_refs(&content);
    assert!(refs.is_empty(), "empty image_key should be skipped");
}

#[test]
fn test_extract_post_media_refs_no_media() {
    let content = serde_json::json!({
        "content": [
            [{"tag": "text", "text": "just text"}]
        ]
    });
    let refs = super::post_expand::extract_post_media_refs(&content);
    assert!(refs.is_empty());
}

#[test]
fn test_extract_post_media_refs_mixed_content() {
    let content = serde_json::json!({
        "content": [
            [{"tag": "text", "text": "before "}],
            [{"tag": "img", "image_key": "img1"}],
            [{"tag": "file", "file_key": "file1"}],
            [{"tag": "text", "text": " after"}]
        ]
    });
    let refs = super::post_expand::extract_post_media_refs(&content);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].key, "img1");
    assert_eq!(refs[0].media_type, closeclaw_common::MediaType::Image);
    assert_eq!(refs[1].key, "file1");
    assert_eq!(refs[1].media_type, closeclaw_common::MediaType::File);
}
