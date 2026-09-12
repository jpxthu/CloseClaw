//! File/audio message tests for Feishu adapter.
//!
//! Extracted from `adapter_tests.rs` to keep file counts under the
//! CONTRIBUTING.md 1000-line hard limit.

use super::*;
use crate::media_store::MediaStore;
use closeclaw_common::{MediaType, MessageType};
use tempfile::TempDir;

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

/// Create a test FeishuAdapter (no real HTTP — only sync methods are exercised).
fn make_test_adapter() -> FeishuAdapter {
    FeishuAdapter::new("test_profile".to_string(), make_test_media_store())
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
// expand_post_content: file tag
// ===========================================================================

#[test]
fn test_expand_post_file_tag() {
    let content = serde_json::json!({
        "content": [[
            {"tag": "file"}
        ]]
    });
    assert_eq!(expand_post_content(&content), "[文件]");
}

// ===========================================================================
// parse_message_event: file / audio types
// ===========================================================================

#[tokio::test]
async fn test_parse_message_event_file_type() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id(
        "file",
        &serde_json::json!({"file_key": "file_xxx", "file_name": "report.pdf"}).to_string(),
        Some("om_msg_002"),
    );
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert_eq!(msg.message_type, MessageType::File);
    // Download fails in unit tests (no HTTP mock) → media unavailable
    assert!(msg.media_refs.is_empty());
    assert!(msg.content.is_empty());
}

#[tokio::test]
async fn test_parse_message_event_audio_type() {
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

// ===========================================================================
// Step 1.2: file/audio filename extraction tests
// ===========================================================================

// --- extract_message_content: JSON content with file_name ---

#[test]
fn test_extract_file_json_with_file_name() {
    let (text, refs, name) = FeishuAdapter::extract_message_content(
        "file",
        &serde_json::json!({"file_key": "file_xxx", "file_name": "report.pdf"}),
    )
    .unwrap();
    assert!(text.is_empty());
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "file_xxx");
    assert_eq!(name.as_deref(), Some("report.pdf"));
}

#[test]
fn test_extract_file_json_without_file_name() {
    let (text, refs, name) = FeishuAdapter::extract_message_content(
        "file",
        &serde_json::json!({"file_key": "file_xxx"}),
    )
    .unwrap();
    assert!(text.is_empty());
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "file_xxx");
    assert!(name.is_none());
}

#[test]
fn test_extract_audio_json_with_file_name() {
    let (text, refs, name) = FeishuAdapter::extract_message_content(
        "audio",
        &serde_json::json!({"file_key": "audio_xxx", "file_name": "voice.ogg"}),
    )
    .unwrap();
    assert!(text.is_empty());
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].key, "audio_xxx");
    assert_eq!(refs[0].media_type, MediaType::Audio);
    assert_eq!(name.as_deref(), Some("voice.ogg"));
}

// --- parse_message_event: malformed XML content returns None ---

#[tokio::test]
async fn test_parse_file_malformed_xml_returns_none() {
    let adapter = make_test_adapter();
    let event = make_message_event_with_id("file", "<file key=\"missing-quote", Some("om_bad_xml"));
    let msg = adapter.parse_message_event(event).await.unwrap();
    assert!(msg.is_none(), "malformed XML should return None, not panic");
}

// --- Fixture test: use p2p-file.json raw event ---

#[tokio::test]
async fn test_fixture_p2p_file_xml_content() {
    // Load the raw fixture and parse it as CLI event.
    let fixture = include_str!("../../../../../tests/fixtures/feishu/cli-poc/p2p-file.json");
    let raw: serde_json::Value = serde_json::from_str(fixture).expect("fixture is valid JSON");
    // Simulate CLI normalization path.
    let event =
        super::process_manager::normalize_cli_event(&raw).expect("fixture normalizes to CLI event");
    assert_eq!(event.event.message_type, "file");
    // Content is XML — parse_message_event should handle fallback.
    let adapter = make_test_adapter();
    let msg = adapter
        .parse_message_event(event)
        .await
        .unwrap()
        .expect("fixture event should parse successfully");
    assert_eq!(msg.message_type, MessageType::File);
    assert!(msg.content.is_empty());
    // Original name extracted from XML; download fails → media unavailable
    assert!(msg.media_refs.is_empty());
}
