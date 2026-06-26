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
        account_id: None,
    };
    let ts_ms = raw.timestamp.timestamp_millis();
    let ctx = make_ctx(raw);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = result
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(!key.is_empty(), "session_key should not be empty");
    // Key format: {timestamp_ms}-{sha256_hex}
    assert!(
        key.starts_with(&format!("{ts_ms}-")),
        "key should start with timestamp prefix: {key}"
    );
    let hash_part = &key[key.find('-').unwrap() + 1..];
    assert_eq!(hash_part.len(), 64, "hash should be 64 hex chars: {key}");
    assert!(
        hash_part.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex: {key}"
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
        account_id: None,
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
        account_id: None,
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
        account_id: None,
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
        account_id: None,
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
        account_id: None,
    };
    let ctx = MessageContext::from_raw(raw);
    let result = router.process(&ctx).await.unwrap().unwrap();
    // No initial_raw → fallback raw with empty fields → empty session_key
    assert!(
        !result.metadata.contains_key("session_key"),
        "no key when raw is absent"
    );
}

#[tokio::test]
async fn test_different_timestamps_produce_different_keys() {
    // Concurrency scenario: same routing fields, different timestamps → different keys
    let router = make_router(DmScope::PerChannelPeer);

    let ts1 = chrono::Utc::now();
    let ts2 = ts1 + chrono::Duration::milliseconds(1);

    let raw1 = RawMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_abc".to_string(),
        peer_id: "oc_xyz".to_string(),
        content: "msg1".to_string(),
        timestamp: ts1,
        message_id: "msg_c1".to_string(),
        account_id: None,
    };
    let raw2 = RawMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_abc".to_string(),
        peer_id: "oc_xyz".to_string(),
        content: "msg2".to_string(),
        timestamp: ts2,
        message_id: "msg_c2".to_string(),
        account_id: None,
    };

    let ctx1 = make_ctx(raw1);
    let ctx2 = make_ctx(raw2);

    let k1 = router
        .process(&ctx1)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let k2 = router
        .process(&ctx2)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    assert_ne!(k1, k2, "different timestamps must produce different keys");
    // Verify each key starts with its own timestamp and has 64-hex-char hash
    assert!(
        k1.starts_with(&format!("{}-", ts1.timestamp_millis())),
        "k1 should start with ts1: {k1}"
    );
    assert!(
        k2.starts_with(&format!("{}-", ts2.timestamp_millis())),
        "k2 should start with ts2: {k2}"
    );
    let h1 = &k1[k1.find('-').unwrap() + 1..];
    let h2 = &k2[k2.find('-').unwrap() + 1..];
    assert_eq!(h1.len(), 64, "k1 hash should be 64 hex chars: {k1}");
    assert_eq!(h2.len(), 64, "k2 hash should be 64 hex chars: {k2}");
    assert!(
        h1.chars().all(|c| c.is_ascii_hexdigit()),
        "k1 hash should be hex: {k1}"
    );
    assert!(
        h2.chars().all(|c| c.is_ascii_hexdigit()),
        "k2 hash should be hex: {k2}"
    );
}

#[tokio::test]
async fn test_same_routing_different_timestamps_different_keys() {
    // Verifies that PerAccountChannelPeer also differentiates by timestamp
    let router = make_router(DmScope::PerAccountChannelPeer);

    let base = chrono::Utc::now();
    let ts_a = base;
    let ts_b = base + chrono::Duration::milliseconds(5);

    let make_raw = |ts: chrono::DateTime<chrono::Utc>| RawMessage {
        platform: "discord".to_string(),
        sender_id: "user_99".to_string(),
        peer_id: "dm_1".to_string(),
        content: "test".to_string(),
        timestamp: ts,
        message_id: "msg_s1".to_string(),
        account_id: None,
    };

    let ctx_a = make_ctx(make_raw(ts_a));
    let ctx_b = make_ctx(make_raw(ts_b));

    let ka = router
        .process(&ctx_a)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let kb = router
        .process(&ctx_b)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    assert_ne!(
        ka, kb,
        "same routing fields with different timestamps must differ"
    );
    // Both should have timestamp prefix and 64-hex-char hash
    assert!(
        ka.starts_with(&format!("{}-", ts_a.timestamp_millis())),
        "ka should start with ts_a: {ka}"
    );
    assert!(
        kb.starts_with(&format!("{}-", ts_b.timestamp_millis())),
        "kb should start with ts_b: {kb}"
    );
    let ha = &ka[ka.find('-').unwrap() + 1..];
    let hb = &kb[kb.find('-').unwrap() + 1..];
    assert_eq!(ha.len(), 64, "ka hash should be 64 hex chars: {ka}");
    assert_eq!(hb.len(), 64, "kb hash should be 64 hex chars: {kb}");
    assert!(
        ha.chars().all(|c| c.is_ascii_hexdigit()),
        "ka hash should be hex: {ka}"
    );
    assert!(
        hb.chars().all(|c| c.is_ascii_hexdigit()),
        "kb hash should be hex: {kb}"
    );
}
