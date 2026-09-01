//! Unit tests for credential isolation (Step 1.4).
//!
//! Covers:
//! - Adapter stores only profile name, not secrets
//! - Profile name is used for CLI commands, not leaked into messages
//! - No credential fields in NormalizedMessage or metadata
//! - Debug log redacts credential fields

use super::adapter::FeishuAdapter;
use crate::media_store::MediaStore;
use crate::IMAdapter;
use std::sync::Arc;
use tempfile::TempDir;

fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

#[test]
fn test_adapter_stores_only_profile_name() {
    let adapter = FeishuAdapter::new("my_lark_profile".to_string(), make_test_media_store());
    assert_eq!(adapter.profile, "my_lark_profile");
    // Verify no credential fields exist on the adapter
    let debug_str = format!("{:?}", adapter);
    assert!(
        !debug_str.contains("app_secret"),
        "adapter debug should not contain app_secret"
    );
    assert!(
        !debug_str.contains("token"),
        "adapter debug should not contain token"
    );
    assert!(
        !debug_str.contains("encrypt_key"),
        "adapter debug should not contain encrypt_key"
    );
}

#[test]
fn test_adapter_default_cli_command() {
    let adapter = FeishuAdapter::new("test".to_string(), make_test_media_store());
    assert_eq!(adapter.cli_command, "lark-cli");
}

#[tokio::test]
async fn test_adapter_profile_not_in_message_content() {
    let adapter = FeishuAdapter::new("secret_profile".to_string(), make_test_media_store());
    let event = super::adapter::FeishuEvent {
        schema: "2.0".to_string(),
        header: super::adapter::FeishuHeader {
            event_id: "ev_cred".to_string(),
            event_type: "im.message.receive_v1".to_string(),
            create_time: "1234567890".to_string(),
            token: "tok".to_string(),
            app_id: "test_app".to_string(),
        },
        event: super::adapter::FeishuMessageEvent {
            message_id: None,
            sender: super::adapter::FeishuSender {
                sender_id: super::adapter::FeishuSenderId {
                    open_id: "ou_sender".to_string(),
                },
                sender_type: "user".to_string(),
            },
            content: serde_json::json!({"text": "hello"}).to_string(),
            chat_id: "oc_chat".to_string(),
            message_type: "text".to_string(),
            thread_id: None,
            root_id: None,
            parent_id: None,
        },
    };
    let msg = adapter.parse_message_event(event).await.unwrap().unwrap();
    assert!(
        !msg.content.contains("secret_profile"),
        "message content should not contain profile name"
    );
    assert!(
        !msg.peer_id.contains("secret_profile"),
        "peer_id should not contain profile name"
    );
    assert!(
        !msg.sender_id.contains("secret_profile"),
        "sender_id should not contain profile name"
    );
}

#[test]
fn test_adapter_metadata_does_not_leak_credentials() {
    let adapter = FeishuAdapter::new("profile_xyz".to_string(), make_test_media_store());
    let meta = adapter.last_metadata.try_lock().unwrap();
    for (key, value) in meta.iter() {
        assert!(
            !value.contains("profile_xyz"),
            "metadata value for key '{}' should not contain profile name",
            key
        );
    }
}

#[tokio::test]
async fn test_validate_signature_always_true() {
    let adapter = FeishuAdapter::new("any_profile".to_string(), make_test_media_store());
    assert!(adapter.validate_signature("any", b"test").await);
    assert!(adapter.validate_signature("", b"").await);
    assert!(
        adapter
            .validate_signature("x".repeat(1000).as_str(), b"y")
            .await
    );
}
