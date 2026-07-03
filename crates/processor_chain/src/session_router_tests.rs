use super::*;
use closeclaw_common::im_plugin::NormalizedMessage;

fn make_router(dm_scope: DmScope) -> SessionRouter {
    SessionRouter::new(dm_scope)
}

fn make_ctx(msg: NormalizedMessage) -> MessageContext {
    MessageContext::from_normalized(msg)
}

#[tokio::test]
async fn test_terminal_session_key_computed() {
    let router = make_router(DmScope::PerChannelPeer);
    let before_ms = chrono::Utc::now().timestamp_millis();
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let after_ms = chrono::Utc::now().timestamp_millis();
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = result
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap();
    assert!(!key.is_empty(), "session_key should not be empty");
    // Key format: {timestamp_ms}-{sha256_hex}
    let ts_prefix: i64 = key[..key.find('-').unwrap()]
        .parse()
        .expect("key prefix should be parseable as i64");
    assert!(
        ts_prefix >= before_ms && ts_prefix <= after_ms + 5,
        "session_key timestamp should reflect system time ({before_ms}..{after_ms}+5ms), got {ts_prefix}: {key}"
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
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_abc".to_string(),
        peer_id: "oc_xyz".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let ctx = make_ctx(msg);
    let r1 = router.process(&ctx).await.unwrap().unwrap();
    let r2 = router.process(&ctx).await.unwrap().unwrap();
    let k1 = r1.metadata.get("session_key").map(|s| s.as_str()).unwrap();
    let k2 = r2.metadata.get("session_key").map(|s| s.as_str()).unwrap();

    // Key format: {timestamp_ms}-{sha256_hex}
    // Timestamp prefix uses system time so it may differ between calls.
    // Compare only the hash part to verify routing determinism.
    assert!(!k1.is_empty(), "session_key must not be empty");
    assert!(!k2.is_empty(), "session_key must not be empty");
    assert!(k1.contains('-'), "key must contain '-' separator: {k1}");
    assert!(k2.contains('-'), "key must contain '-' separator: {k2}");

    let hash1 = &k1[k1.find('-').unwrap() + 1..];
    let hash2 = &k2[k2.find('-').unwrap() + 1..];
    assert_eq!(hash1.len(), 64, "hash must be 64 hex chars: {k1}");
    assert!(
        hash1.chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be hex: {k1}"
    );
    assert_eq!(hash1, hash2, "same routing fields must produce same hash");
}

#[tokio::test]
async fn test_missing_peer_id_yields_empty_key() {
    let router = make_router(DmScope::PerChannelPeer);
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "u1".to_string(),
        peer_id: String::new(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = result
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap_or("");
    assert!(key.is_empty(), "missing peer_id should yield empty key");
}

#[tokio::test]
async fn test_dm_scope_affects_key() {
    let r1 = make_router(DmScope::PerPeer);
    let r2 = make_router(DmScope::PerChannelPeer);
    let msg = NormalizedMessage {
        platform: "discord".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "dm_42".to_string(),
        content: "test".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let ctx = make_ctx(msg);
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
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let mut ctx = make_ctx(msg);
    ctx.metadata
        .insert("existing_key".to_string(), "existing_value".to_string());
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
async fn test_fallback_when_no_initial_normalized() {
    let router = make_router(DmScope::PerChannelPeer);
    let msg = NormalizedMessage {
        platform: String::new(),
        sender_id: String::new(),
        peer_id: String::new(),
        content: String::new(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let ctx = MessageContext::from_normalized(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    // No initial_raw → fallback raw with empty fields → empty session_key
    assert!(
        !result.metadata.contains_key("session_key"),
        "no key when raw is absent"
    );
}

#[tokio::test]
async fn test_system_time_used_for_session_key() {
    // SessionRouter uses system time, not message timestamp
    let router = make_router(DmScope::PerChannelPeer);
    let before_ms = chrono::Utc::now().timestamp_millis();

    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_abc".to_string(),
        peer_id: "oc_xyz".to_string(),
        content: "msg1".to_string(),
        // Message timestamp is in the past — should NOT be used
        timestamp: 1_577_836_800_000,
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let after_ms = chrono::Utc::now().timestamp_millis();
    let ctx = make_ctx(msg);

    let key = router
        .process(&ctx)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap()
        .to_string();

    // Key prefix must be between before_ms and after_ms, not 2020
    let ts_prefix: i64 = key[..key.find('-').unwrap()]
        .parse()
        .expect("key prefix should be parseable as i64");
    assert!(
        ts_prefix >= before_ms && ts_prefix <= after_ms + 5,
        "session_key timestamp should reflect system time ({before_ms}..{after_ms}+5ms), got {ts_prefix}: {key}"
    );
    let hash = &key[key.find('-').unwrap() + 1..];
    assert_eq!(hash.len(), 64, "hash should be 64 hex chars: {key}");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex: {key}"
    );
}

#[tokio::test]
async fn test_per_account_channel_peer_uses_system_time() {
    // Verifies that PerAccountChannelPeer also uses system time
    let router = make_router(DmScope::PerAccountChannelPeer);
    let before_ms = chrono::Utc::now().timestamp_millis();

    let msg = NormalizedMessage {
        platform: "discord".to_string(),
        sender_id: "user_99".to_string(),
        peer_id: "dm_1".to_string(),
        content: "test".to_string(),
        // Past message timestamp — should NOT appear in session_key
        timestamp: 1_577_836_800_000,
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let after_ms = chrono::Utc::now().timestamp_millis();
    let ctx = make_ctx(msg);

    let key = router
        .process(&ctx)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap()
        .to_string();

    let ts_prefix: i64 = key[..key.find('-').unwrap()]
        .parse()
        .expect("key prefix should be parseable as i64");
    assert!(
        ts_prefix >= before_ms && ts_prefix <= after_ms + 5,
        "session_key timestamp should reflect system time ({before_ms}..{after_ms}+5ms), got {ts_prefix}: {key}"
    );
    let hash = &key[key.find('-').unwrap() + 1..];
    assert_eq!(hash.len(), 64, "hash should be 64 hex chars: {key}");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex: {key}"
    );
}

// -----------------------------------------------------------------------
// account_id tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_different_account_ids_produce_different_session_keys() {
    // Different account_id values must produce different session_key hashes
    let router = make_router(DmScope::PerAccountChannelPeer);

    let msg_a = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: "account_1".to_string(),
    };
    let msg_b = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: "account_2".to_string(),
    };

    let ctx_a = make_ctx(msg_a);
    let ctx_b = make_ctx(msg_b);

    let key_a = router
        .process(&ctx_a)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap()
        .to_string();
    let key_b = router
        .process(&ctx_b)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap()
        .to_string();

    assert_ne!(
        key_a, key_b,
        "different account_id must produce different session_key"
    );
    // Both must have valid format
    assert!(
        key_a.contains('-'),
        "key_a should contain timestamp separator: {key_a}"
    );
    assert!(
        key_b.contains('-'),
        "key_b should contain timestamp separator: {key_b}"
    );
}

#[tokio::test]
async fn test_account_id_none_vs_some_produce_different_keys() {
    // account_id=None vs account_id=Some(...) must produce different keys
    let router = make_router(DmScope::PerAccountChannelPeer);

    let msg_none = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let msg_some = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: "tenant_x".to_string(),
    };

    let ctx_none = make_ctx(msg_none);
    let ctx_some = make_ctx(msg_some);

    let key_none = router
        .process(&ctx_none)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap()
        .to_string();
    let key_some = router
        .process(&ctx_some)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap()
        .to_string();

    assert_ne!(
        key_none, key_some,
        "account_id None vs Some must produce different session_key"
    );
}

#[tokio::test]
async fn test_account_id_read_from_raw_message() {
    // Verify account_id is read from RawMessage, not from ctx.metadata
    let router = make_router(DmScope::PerAccountChannelPeer);
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: "tenant_42".to_string(),
    };
    let mut ctx = make_ctx(msg);
    // Even if metadata has a different account_id, NormalizedMessage should win
    ctx.metadata
        .insert("account_id".to_string(), "metadata_account".to_string());
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = result
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap();
    assert!(!key.is_empty(), "session_key should be set");
    // Verify key hash differs from one computed with "metadata_account"
    let msg_meta = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: "metadata_account".to_string(),
    };
    let ctx_meta = make_ctx(msg_meta);
    let result_meta = router.process(&ctx_meta).await.unwrap().unwrap();
    let key_meta = result_meta
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .unwrap();
    // They should differ because account_id comes from NormalizedMessage
    assert_ne!(
        key, key_meta,
        "account_id should be read from NormalizedMessage, not metadata"
    );
}
