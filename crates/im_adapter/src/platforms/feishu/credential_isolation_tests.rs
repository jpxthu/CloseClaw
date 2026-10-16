//! Unit tests for credential isolation (Step 1.4).
//!
//! Covers:
//! - Adapter stores only profile name, not secrets
//! - Profile name is used for CLI commands, not leaked into messages
//! - No credential fields in NormalizedMessage or metadata
//! - Debug log redacts credential fields
//! - Debug output whitelist keeps diagnostic value (profile, cli_command)
//!   without internal implementation details (reqwest internals, etc.)

use super::make_adapter_with_profile;
use crate::IMAdapter;

#[test]
fn test_adapter_stores_only_profile_name() {
    let adapter = make_adapter_with_profile("my_lark_profile");
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
    let adapter = make_adapter_with_profile("test");
    assert_eq!(adapter.cli_command, "lark-cli");
}

#[test]
fn test_adapter_debug_contains_profile_and_cli_command() {
    let mut adapter = make_adapter_with_profile("my_lark_profile");
    adapter.cli_command = "/opt/tools/lark-cli".to_string();
    let debug_str = format!("{:?}", adapter);
    assert!(
        debug_str.contains("my_lark_profile"),
        "debug output should contain profile name for diagnostics, got: {debug_str}"
    );
    assert!(
        debug_str.contains("/opt/tools/lark-cli"),
        "debug output should contain cli_command value for diagnostics, got: {debug_str}"
    );
}

#[test]
fn test_adapter_debug_excludes_internal_impl_markers() {
    let adapter = make_adapter_with_profile("impl_isolation");
    let debug_str = format!("{:?}", adapter);
    assert!(
        !debug_str.contains("proxies"),
        "debug output should not contain reqwest proxy internals, got: {debug_str}"
    );
    assert!(
        !debug_str.contains("NoProxy"),
        "debug output should not contain reqwest NoProxy table, got: {debug_str}"
    );
    assert!(
        !debug_str.contains("auto_sys_proxy"),
        "debug output should not contain reqwest auto_sys_proxy flag, got: {debug_str}"
    );
}

#[test]
fn test_adapter_debug_placeholders_are_bare_type_names() {
    let adapter = make_adapter_with_profile("placeholder_bind");
    let debug_str = format!("{:?}", adapter);
    // Bind exact placeholder strings so internal type names cannot drift silently.
    assert!(
        debug_str.contains("<Arc<MediaStore>>"),
        "debug output should bind media_store type-name placeholder, got: {debug_str}"
    );
    assert!(
        debug_str.contains("<reqwest::Client>"),
        "debug output should bind http_client type-name placeholder, got: {debug_str}"
    );
    assert!(
        debug_str.contains("<Arc<Mutex<HashMap>>>"),
        "debug output should bind last_metadata type-name placeholder, got: {debug_str}"
    );
    assert!(
        debug_str.contains("<Arc<Mutex<EventDeduplicator>>>"),
        "debug output should bind event_dedup type-name placeholder, got: {debug_str}"
    );
    // Placeholders must render bare (no quote wrapping from string-literal Debug).
    assert!(
        debug_str.contains("media_store: <Arc<MediaStore>>"),
        "media_store placeholder should render without quotes, got: {debug_str}"
    );
    assert!(
        debug_str.contains("http_client: <reqwest::Client>"),
        "http_client placeholder should render without quotes, got: {debug_str}"
    );
    assert!(
        !debug_str.contains("\"<"),
        "no placeholder should be quote-wrapped, got: {debug_str}"
    );
}

#[test]
fn test_adapter_debug_workspace_dir_none_format() {
    let adapter = make_adapter_with_profile("ws_none_profile");
    assert!(adapter.workspace_dir.is_none());
    let debug_str = format!("{:?}", adapter);
    assert!(
        debug_str.contains("workspace_dir: None"),
        "debug output should render workspace_dir as None, got: {debug_str}"
    );
    assert!(
        !debug_str.contains('\n'),
        "debug output should be single-line, got: {debug_str}"
    );
}

#[tokio::test]
async fn test_adapter_profile_not_in_message_content() {
    let adapter = make_adapter_with_profile("secret_profile");
    let event = super::lark_types::FeishuEvent {
        schema: "2.0".to_string(),
        header: super::lark_types::FeishuHeader {
            event_id: "ev_cred".to_string(),
            event_type: "im.message.receive_v1".to_string(),
            create_time: "1234567890".to_string(),
            token: "tok".to_string(),
            app_id: "test_app".to_string(),
        },
        event: super::lark_types::FeishuMessageEvent {
            message_id: None,
            sender: super::lark_types::FeishuSender {
                sender_id: super::lark_types::FeishuSenderId {
                    open_id: "ou_sender".to_string(),
                },
                sender_type: "user".to_string(),
            },
            content: serde_json::json!({"text": "hello"}).to_string(),
            chat_id: "oc_chat".to_string(),
            chat_type: None,
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
    let adapter = make_adapter_with_profile("profile_xyz");
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
    let adapter = make_adapter_with_profile("any_profile");
    assert!(adapter.validate_signature("any", b"test").await);
    assert!(adapter.validate_signature("", b"").await);
    assert!(
        adapter
            .validate_signature("x".repeat(1000).as_str(), b"y")
            .await
    );
}
