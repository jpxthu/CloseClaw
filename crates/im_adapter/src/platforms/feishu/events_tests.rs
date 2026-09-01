//! Unit tests for reaction.created and bot.added event parsing.

use super::adapter::FeishuAdapter;
use super::adapter::FEISHU_API_BASE;
use crate::media_store::MediaStore;
use crate::IMAdapter;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

/// Create a test FeishuAdapter (no real HTTP — only sync methods are exercised).
fn make_test_adapter() -> FeishuAdapter {
    let http_client = reqwest::Client::new();
    FeishuAdapter {
        app_id: "test_app_id".to_string(),
        app_secret: "test_secret".to_string(),
        verification_token: "test_token".to_string(),
        http_client,
        cached_token: Arc::new(tokio::sync::Mutex::new(None)),
        base_url: FEISHU_API_BASE.to_string(),
        last_metadata: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        media_store: make_test_media_store(),
        max_download_size_bytes: u64::MAX,
        workspace_dir: None,
    }
}

// ===========================================================================
// reaction.created event tests
// ===========================================================================

/// Build a minimal `reaction.created` webhook payload.
fn make_reaction_payload(message_id: &str, open_id: &str, emoji_type: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_test_reaction",
            "event_type": "reaction.created",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {
            "message_id": message_id,
            "operator": {
                "operator_id": {
                    "open_id": open_id
                },
                "operator_type": "user"
            },
            "reaction_type": {
                "emoji_type": emoji_type
            }
        }
    });
    serde_json::to_vec(&payload).unwrap()
}

/// reaction.created: returns Ok(None) — not a NormalizedMessage.
#[tokio::test]
async fn test_parse_inbound_reaction_created_returns_none() {
    let adapter = make_test_adapter();
    let payload = make_reaction_payload("om_test_msg_001", "ou_user_abc", "THUMBSUP");
    let result = adapter.parse_inbound(&payload).await.unwrap();
    assert!(result.is_none());
}

/// reaction.created: missing reaction_type — graceful parse error (no panic).
#[tokio::test]
async fn test_parse_inbound_reaction_created_missing_reaction_type() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_test_no_reaction_type",
            "event_type": "reaction.created",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {
            "message_id": "om_msg_no_rt",
            "operator": {
                "open_id": "ou_user_no_rt",
                "operator_type": "user"
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let result: Result<Option<_>, _> = adapter.parse_inbound(&bytes).await;
    assert!(
        result.is_err(),
        "missing reaction_type should cause parse error"
    );
}

/// reaction.created: missing message_id — graceful parse error (no panic).
#[tokio::test]
async fn test_parse_inbound_reaction_created_missing_message_id() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_test_no_msg_id",
            "event_type": "reaction.created",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {
            "operator": {
                "open_id": "ou_user_no_msg",
                "operator_type": "user"
            },
            "reaction_type": {
                "emoji_type": "THUMBSUP"
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let result: Result<Option<_>, _> = adapter.parse_inbound(&bytes).await;
    assert!(
        result.is_err(),
        "missing message_id should cause parse error"
    );
}

/// reaction.created: operator at top level (open_id field) — graceful handling.
#[tokio::test]
async fn test_parse_inbound_reaction_created_with_top_level_open_id() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_test_min",
            "event_type": "reaction.created",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {
            "message_id": "om_msg_min",
            "operator": {
                "open_id": "ou_user_min",
                "operator_type": "user"
            },
            "reaction_type": {
                "emoji_type": "CLAP"
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let result = adapter.parse_inbound(&bytes).await.unwrap();
    assert!(result.is_none());
}

// ===========================================================================
// bot.added event tests
// ===========================================================================

/// Build a minimal `bot.added` webhook payload.
fn make_bot_added_payload(chat_id: &str, bot_open_id: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_test_bot_added",
            "event_type": "bot.added",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {
            "chat_id": chat_id,
            "bot": {
                "open_id": bot_open_id
            }
        }
    });
    serde_json::to_vec(&payload).unwrap()
}

/// bot.added: returns Ok(None) — not a NormalizedMessage.
#[tokio::test]
async fn test_parse_inbound_bot_added_returns_none() {
    let adapter = make_test_adapter();
    let payload = make_bot_added_payload("oc_group_chat_001", "ou_bot_xyz");
    let result = adapter.parse_inbound(&payload).await.unwrap();
    assert!(result.is_none());
}

/// bot.added: missing chat_id field — graceful handling.
#[tokio::test]
async fn test_parse_inbound_bot_added_missing_chat_id() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_test_bot_missing",
            "event_type": "bot.added",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {
            "bot": {
                "open_id": "ou_bot_abc"
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let result: Result<Option<_>, _> = adapter.parse_inbound(&bytes).await;
    assert!(result.is_err(), "missing chat_id should cause parse error");
}

/// bot.added: missing bot.open_id field — graceful handling.
#[tokio::test]
async fn test_parse_inbound_bot_added_missing_bot_open_id() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_test_bot_no_openid",
            "event_type": "bot.added",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {
            "chat_id": "oc_chat_def"
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let result: Result<Option<_>, _> = adapter.parse_inbound(&bytes).await;
    assert!(
        result.is_err(),
        "missing bot.open_id should cause parse error"
    );
}

/// card.action.trigger: still returns Ok(None) — no regression.
#[tokio::test]
async fn test_parse_inbound_card_action_still_returns_none() {
    let adapter = make_test_adapter();
    let payload = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_id": "ev_card",
            "event_type": "card.action.trigger",
            "create_time": "1700000000000",
            "token": "test_token",
            "app_id": "cli_test"
        },
        "event": {}
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let result = adapter.parse_inbound(&bytes).await.unwrap();
    assert!(result.is_none());
}
