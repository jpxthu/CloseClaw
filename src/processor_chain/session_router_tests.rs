use super::*;
use crate::processor_chain::context::RawMessage;

fn make_router(dm_scope: DmScope) -> SessionRouter {
    SessionRouter::new(dm_scope)
}

fn make_ctx(raw: RawMessage) -> MessageContext {
    MessageContext::from_raw(raw)
}

#[tokio::test]
async fn test_terminal_session_key_computed() {
    let router = make_router(DmScope::PerChannelPeer);
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_1".to_string(),
    };
    let ctx = make_ctx(raw);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = result
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(!key.is_empty(), "session_key should not be empty");
    assert!(key.contains("1000"), "key should contain sender_id: {key}");
    assert!(key.contains("cli"), "key should contain peer_id: {key}");
    assert!(
        key.contains("terminal"),
        "key should contain platform: {key}"
    );
}

#[tokio::test]
async fn test_deterministic_key() {
    let router = make_router(DmScope::PerAccountChannelPeer);
    let raw = RawMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_abc".to_string(),
        peer_id: "oc_xyz".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_d".to_string(),
    };
    let ctx = make_ctx(raw);
    let r1 = router.process(&ctx).await.unwrap().unwrap();
    let r2 = router.process(&ctx).await.unwrap().unwrap();
    let k1 = r1.metadata.get("session_key").unwrap();
    let k2 = r2.metadata.get("session_key").unwrap();
    assert_eq!(k1, k2, "same input must produce same session_key");
}

#[tokio::test]
async fn test_missing_peer_id_yields_empty_key() {
    let router = make_router(DmScope::PerChannelPeer);
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "u1".to_string(),
        peer_id: String::new(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_e".to_string(),
    };
    let ctx = make_ctx(raw);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = result
        .metadata
        .get("session_key")
        .map(|v| v.as_str().unwrap_or(""))
        .unwrap_or("");
    assert!(key.is_empty(), "missing peer_id should yield empty key");
}

#[tokio::test]
async fn test_dm_scope_affects_key() {
    let r1 = make_router(DmScope::PerPeer);
    let r2 = make_router(DmScope::PerChannelPeer);
    let raw = RawMessage {
        platform: "discord".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "dm_42".to_string(),
        content: "test".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_f".to_string(),
    };
    let ctx = make_ctx(raw);
    let k1 = r1
        .process(&ctx)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .unwrap()
        .clone();
    let k2 = r2
        .process(&ctx)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .unwrap()
        .clone();
    assert_ne!(k1, k2, "different DmScope should produce different keys");
}

#[tokio::test]
async fn test_metadata_preserves_upstream() {
    let router = make_router(DmScope::PerChannelPeer);
    let raw = RawMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now(),
        message_id: "msg_g".to_string(),
    };
    let mut ctx = make_ctx(raw);
    ctx.metadata.insert(
        "existing_key".to_string(),
        serde_json::json!("existing_value"),
    );
    let result = router.process(&ctx).await.unwrap().unwrap();
    assert_eq!(
        result.metadata.get("existing_key").unwrap(),
        "existing_value"
    );
    assert!(result.metadata.contains_key("session_key"));
    assert!(result.metadata.contains_key("platform"));
    assert!(result.metadata.contains_key("sender_id"));
    assert!(result.metadata.contains_key("peer_id"));
}

#[tokio::test]
async fn test_fallback_when_no_initial_raw() {
    let router = make_router(DmScope::PerChannelPeer);
    let raw = RawMessage {
        platform: String::new(),
        sender_id: String::new(),
        peer_id: String::new(),
        content: String::new(),
        timestamp: chrono::Utc::now(),
        message_id: String::new(),
    };
    let ctx = MessageContext::from_raw(raw);
    let result = router.process(&ctx).await.unwrap().unwrap();
    // No initial_raw → fallback raw with empty fields → empty session_key
    assert!(
        !result.metadata.contains_key("session_key"),
        "no key when raw is absent"
    );
}
