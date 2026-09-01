//! Unit tests for Feishu adapter: sticker message handling.

use super::*;
use crate::media_store::MediaStore;
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

#[test]
fn test_extract_sticker_with_emoji_type() {
    let (text, media) =
        FeishuAdapter::extract_message_content("sticker", &serde_json::json!({"emoji_type": "OK"}))
            .unwrap();
    assert_eq!(text, "[OK]");
    assert!(media.is_empty());
}

#[test]
fn test_extract_sticker_without_emoji_type() {
    let (text, media) =
        FeishuAdapter::extract_message_content("sticker", &serde_json::json!({})).unwrap();
    assert_eq!(text, "[]");
    assert!(media.is_empty());
}

#[test]
fn test_extract_sticker_empty_emoji_type() {
    let (text, media) =
        FeishuAdapter::extract_message_content("sticker", &serde_json::json!({"emoji_type": ""}))
            .unwrap();
    assert_eq!(text, "[]");
    assert!(media.is_empty());
}

#[tokio::test]
async fn test_parse_sticker_with_emoji_type() {
    let a = make_test_adapter();
    let e = make_message_event(
        "sticker",
        &serde_json::json!({"emoji_type": "THUMBSUP"}).to_string(),
    );
    let msg = a.parse_message_event(e).await.unwrap().unwrap();
    assert_eq!(msg.content, "[THUMBSUP]");
    assert!(msg.media_refs.is_empty());
}

#[tokio::test]
async fn test_parse_sticker_without_emoji_type() {
    let a = make_test_adapter();
    let e = make_message_event("sticker", &serde_json::json!({}).to_string());
    let msg = a.parse_message_event(e).await.unwrap().unwrap();
    assert_eq!(msg.content, "[]");
    assert!(msg.media_refs.is_empty());
}
