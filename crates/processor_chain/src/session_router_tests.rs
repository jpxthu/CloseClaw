use super::*;
use crate::processor_chain::context::metadata_str;
use closeclaw_common::im_plugin::NormalizedMessage;
use std::sync::{Arc, Mutex};
use tracing::field::Field;
use tracing::Subscriber;

/// Fixed past message timestamp (2020-01-01T00:00:00Z in ms) used to prove
/// that `msg.timestamp` does not leak into the computed session_key.
const FIXED_PAST_TS_MS: i64 = 1_577_836_800_000;

/// Base timestamp (ms) shared by the deterministic-key, timestamp,
/// routing-sensitivity, and fixed-vector tests below.
const BASE_TS_MS: i64 = 1_700_000_000_123;

fn make_router() -> SessionRouter {
    SessionRouter::new()
}

fn make_ctx(msg: NormalizedMessage) -> MessageContext {
    MessageContext::from_normalized(msg)
}

#[tokio::test]
async fn test_terminal_session_key_computed() {
    let router = make_router();
    let before_ms = chrono::Utc::now().timestamp_millis();
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let after_ms = chrono::Utc::now().timestamp_millis();
    let key = session_key_of(&result).unwrap();
    assert!(!key.is_empty(), "session_key should not be empty");
    assert_key_ts_in_window(key, before_ms, after_ms);
}

/// Build a `MessageContext` for session_key checks. `msg.timestamp` is a
/// fixed past value that must NOT influence the computed `session_key`.
fn fixed_ts_ctx(sender: &str) -> MessageContext {
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: sender.to_string(),
        peer_id: "oc_xyz".to_string(),
        content: "hi".to_string(),
        timestamp: FIXED_PAST_TS_MS,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    make_ctx(msg)
}

/// Parse a `{timestamp_ms}-{64 hex}` session_key, asserting its structure.
fn split_key(key: &str) -> (i64, &str) {
    let pos = key.find('-').expect("key must contain '-' separator");
    let ts: i64 = key[..pos]
        .parse()
        .expect("key timestamp prefix must parse as i64");
    let hash = &key[pos + 1..];
    assert_eq!(hash.len(), 64, "hash must be 64 hex chars: {key}");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be hex: {key}"
    );
    (ts, hash)
}

/// Assert the key's timestamp prefix falls within `[before_ms, after_ms]`.
/// `before_ms` must be sampled before `process()` and `after_ms` after it,
/// so the window covers every system clock read inside `process()`.
fn assert_key_ts_in_window(key: &str, before_ms: i64, after_ms: i64) {
    let (ts_prefix, _) = split_key(key);
    assert!(
        ts_prefix >= before_ms && ts_prefix <= after_ms,
        "session_key timestamp should reflect system time \
         (window {before_ms}..{after_ms}ms), got {ts_prefix}: {key}"
    );
}

/// Extract `metadata.session_key` as `&str`; `None` when the key is absent.
fn session_key_of(msg: &ProcessedMessage) -> Option<&str> {
    metadata_str(&msg.metadata, "session_key")
}

#[test]
fn test_deterministic_key() {
    // Determinism: identical routing fields + identical timestamp_ms -> same key.
    let ts: i64 = BASE_TS_MS;
    let key1 = SessionRouter::compute_key("ou_abc", "oc_xyz", "feishu", None, ts);
    let key2 = SessionRouter::compute_key("ou_abc", "oc_xyz", "feishu", None, ts);
    assert!(!key1.is_empty(), "session_key must not be empty");
    assert_eq!(key1, key2, "same inputs must produce identical session_key");
    let (ts1, _) = split_key(&key1);
    assert_eq!(ts1, ts, "key prefix must equal timestamp_ms: {key1}");
}

#[test]
fn test_timestamp_participation() {
    // Time participation: timestamp_ms changes prefix and hash (anti-collision).
    let ts: i64 = BASE_TS_MS;
    let key1 = SessionRouter::compute_key("ou_abc", "oc_xyz", "feishu", None, ts);
    let key_later = SessionRouter::compute_key("ou_abc", "oc_xyz", "feishu", None, ts + 1);
    let (ts1, hash1) = split_key(&key1);
    let (ts_later, hash_later) = split_key(&key_later);
    assert_eq!(ts1, ts, "prefix must track its timestamp_ms: {key1}");
    assert_eq!(
        ts_later,
        ts + 1,
        "prefix must track its timestamp_ms: {key_later}"
    );
    assert_ne!(
        hash_later, hash1,
        "timestamp_ms must participate in hash input"
    );
}

#[test]
fn test_routing_sensitivity() {
    // Routing sensitivity: any differing routing field -> different hash.
    let ts: i64 = BASE_TS_MS;
    let key = SessionRouter::compute_key("ou_abc", "oc_xyz", "feishu", None, ts);
    let (_, hash) = split_key(&key);

    let key_sender = SessionRouter::compute_key("ou_other", "oc_xyz", "feishu", None, ts);
    let (_, hash_sender) = split_key(&key_sender);
    assert_ne!(hash_sender, hash, "sender change must change the hash");

    let key_peer = SessionRouter::compute_key("ou_abc", "oc_other", "feishu", None, ts);
    let (_, hash_peer) = split_key(&key_peer);
    assert_ne!(hash_peer, hash, "peer change must change the hash");

    let key_channel = SessionRouter::compute_key("ou_abc", "oc_xyz", "discord", None, ts);
    let (_, hash_channel) = split_key(&key_channel);
    assert_ne!(hash_channel, hash, "channel change must change the hash");
}

#[tokio::test]
async fn test_process_key_matches_formula() {
    // process() output satisfies the design formula end to end.
    let router = make_router();
    let out = router
        .process(&fixed_ts_ctx("ou_abc"))
        .await
        .unwrap()
        .unwrap();
    let key = session_key_of(&out).expect("session_key must be set");
    let (key_ts, _) = split_key(key);
    assert_ne!(
        key_ts, FIXED_PAST_TS_MS,
        "msg.timestamp must not leak into session_key prefix: {key}"
    );
    let expect = closeclaw_common::session_key::compute_session_key(
        "feishu", "ou_abc", "oc_xyz", None, key_ts,
    );
    assert_eq!(key, expect, "process() key must match design formula");
}

/// Fixed-vector guard: the hex values below are precomputed with an
/// independent tool (python3 hashlib), NOT derived from compute_session_key,
/// so any change to the formula itself (separator, field order, "default"
/// literal, hash algorithm) breaks these assertions. Recompute commands
/// (each is one shell command spanning two lines; their timestamp component
/// 1700000000123 is `BASE_TS_MS`):
///   python3 -c "import hashlib; print(hashlib.sha256(
///       b'feishu:ou_abc:oc_xyz:default:1700000000123').hexdigest())"
///   python3 -c "import hashlib; print(hashlib.sha256(
///       b'feishu:ou_abc:oc_xyz:tenant_x:1700000000123').hexdigest())"
#[test]
fn test_session_key_fixed_vector() {
    let ts: i64 = BASE_TS_MS;

    // account_id: None -> literal "default" participates in hash input
    let key = SessionRouter::compute_key("ou_abc", "oc_xyz", "feishu", None, ts);
    assert_eq!(
        key, "1700000000123-f255c84b77b426f609f9a8fad1a10daba28047ada6c606734103351e05d2ddc1",
        "session_key must match the independently precomputed vector (default)"
    );

    // account_id: Some -> account literal participates in hash input
    let key_some = SessionRouter::compute_key("ou_abc", "oc_xyz", "feishu", Some("tenant_x"), ts);
    assert_eq!(
        key_some, "1700000000123-e104f2e55d3037c1e893589ea706a4828261fa96e19f90d67779862b11476677",
        "session_key must match the independently precomputed vector (tenant_x)"
    );
}

#[tokio::test]
async fn test_missing_peer_id_yields_empty_key() {
    let router = make_router();
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "u1".to_string(),
        peer_id: String::new(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = session_key_of(&result).unwrap_or("");
    assert!(key.is_empty(), "missing peer_id should yield empty key");
}

#[tokio::test]
async fn test_metadata_preserves_upstream() {
    let router = make_router();
    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "1000".to_string(),
        peer_id: "cli".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
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
    let router = make_router();
    let msg = NormalizedMessage {
        platform: String::new(),
        sender_id: String::new(),
        peer_id: String::new(),
        content: String::new(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
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
    let router = make_router();
    let before_ms = chrono::Utc::now().timestamp_millis();

    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_abc".to_string(),
        peer_id: "oc_xyz".to_string(),
        content: "msg1".to_string(),
        // Message timestamp is in the past — should NOT be used
        timestamp: FIXED_PAST_TS_MS,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);

    let out = router.process(&ctx).await.unwrap().unwrap();
    let after_ms = chrono::Utc::now().timestamp_millis();
    let key = session_key_of(&out).unwrap();
    assert_key_ts_in_window(key, before_ms, after_ms);
}

#[tokio::test]
async fn test_per_account_channel_peer_uses_system_time() {
    let router = make_router();
    let before_ms = chrono::Utc::now().timestamp_millis();

    let msg = NormalizedMessage {
        platform: "discord".to_string(),
        sender_id: "user_99".to_string(),
        peer_id: "dm_1".to_string(),
        content: "test".to_string(),
        // Past message timestamp — should NOT appear in session_key
        timestamp: FIXED_PAST_TS_MS,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);

    let out = router.process(&ctx).await.unwrap().unwrap();
    let after_ms = chrono::Utc::now().timestamp_millis();
    let key = session_key_of(&out).unwrap();
    assert_key_ts_in_window(key, before_ms, after_ms);
}

// -----------------------------------------------------------------------
// account_id tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_different_account_ids_produce_different_session_keys() {
    let router = make_router();

    let msg_a = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: "account_1".to_string(),
        ..Default::default()
    };
    let msg_b = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: "account_2".to_string(),
        ..Default::default()
    };

    let ctx_a = make_ctx(msg_a);
    let ctx_b = make_ctx(msg_b);

    let out_a = router.process(&ctx_a).await.unwrap().unwrap();
    let key_a = session_key_of(&out_a).unwrap();
    let out_b = router.process(&ctx_b).await.unwrap().unwrap();
    let key_b = session_key_of(&out_b).unwrap();

    assert_ne!(
        key_a, key_b,
        "different account_id must produce different session_key"
    );
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
    let router = make_router();

    let msg_none = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let msg_some = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "msg".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: "tenant_x".to_string(),
        ..Default::default()
    };

    let ctx_none = make_ctx(msg_none);
    let ctx_some = make_ctx(msg_some);

    let out_none = router.process(&ctx_none).await.unwrap().unwrap();
    let key_none = session_key_of(&out_none).unwrap();
    let out_some = router.process(&ctx_some).await.unwrap().unwrap();
    let key_some = session_key_of(&out_some).unwrap();

    assert_ne!(
        key_none, key_some,
        "account_id None vs Some must produce different session_key"
    );
}

#[tokio::test]
async fn test_account_id_read_from_raw_message() {
    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: "tenant_42".to_string(),
        ..Default::default()
    };
    let mut ctx = make_ctx(msg);
    ctx.metadata
        .insert("account_id".to_string(), "metadata_account".to_string());
    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = session_key_of(&result).unwrap();
    assert!(!key.is_empty(), "session_key should be set");
    let msg_meta = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_group".to_string(),
        content: "hi".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: "metadata_account".to_string(),
        ..Default::default()
    };
    let ctx_meta = make_ctx(msg_meta);
    let result_meta = router.process(&ctx_meta).await.unwrap().unwrap();
    let key_meta = session_key_of(&result_meta).unwrap();
    assert_ne!(
        key, key_meta,
        "account_id should be read from NormalizedMessage, not metadata"
    );
}

// -----------------------------------------------------------------------
// warn log verification tests
// -----------------------------------------------------------------------

struct WarnCapture {
    events: Arc<Mutex<Vec<String>>>,
}

impl WarnCapture {
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                events: events.clone(),
            },
            events,
        )
    }
}

impl Subscriber for WarnCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() == tracing::Level::WARN {
            struct MsgVisitor<'a> {
                msg: &'a mut String,
            }
            impl<'a> tracing::field::Visit for MsgVisitor<'a> {
                fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                    if field.name() == "message" {
                        self.msg.push_str(&format!("{:?}", value));
                    }
                }
            }
            let mut msg = String::new();
            event.record(&mut MsgVisitor { msg: &mut msg });
            self.events.lock().unwrap().push(msg);
        }
    }

    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

#[tokio::test]
async fn test_warn_log_when_from_empty() {
    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: String::new(), // from is empty
        peer_id: "oc_chat".to_string(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);

    let (subscriber, events) = WarnCapture::new();
    let _guard = tracing::subscriber::set_default(subscriber);

    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = session_key_of(&result).unwrap_or("");
    assert!(
        key.is_empty(),
        "session_key should be empty when from is empty"
    );

    let captured = events.lock().unwrap();
    assert!(
        !captured.is_empty(),
        "warn log should be emitted when from is empty"
    );
    assert!(
        captured[0].contains("SessionRouter"),
        "warn should mention SessionRouter: {}",
        captured[0]
    );
}

#[tokio::test]
async fn test_warn_log_when_to_empty() {
    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: String::new(), // to is empty
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);

    let (subscriber, events) = WarnCapture::new();
    let _guard = tracing::subscriber::set_default(subscriber);

    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = session_key_of(&result).unwrap_or("");
    assert!(
        key.is_empty(),
        "session_key should be empty when to is empty"
    );

    let captured = events.lock().unwrap();
    assert!(
        !captured.is_empty(),
        "warn log should be emitted when to is empty"
    );
    assert!(
        captured[0].contains("SessionRouter"),
        "warn should mention SessionRouter: {}",
        captured[0]
    );
}

#[tokio::test]
async fn test_no_warn_log_when_both_present() {
    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_chat".to_string(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);

    let (subscriber, events) = WarnCapture::new();
    let _guard = tracing::subscriber::set_default(subscriber);

    let result = router.process(&ctx).await.unwrap().unwrap();
    let key = session_key_of(&result).unwrap_or("");
    assert!(
        !key.is_empty(),
        "session_key should be non-empty when both present"
    );

    let captured = events.lock().unwrap();
    assert!(
        captured.is_empty(),
        "no warn log should be emitted when both from and to are present, got: {:?}",
        *captured
    );
}

// -----------------------------------------------------------------------
// message_type injection tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_message_type_injected_into_metadata() {
    use closeclaw_common::im_plugin::MessageType;

    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_chat".to_string(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: MessageType::Text,
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let mt = metadata_str(&result.metadata, "message_type").unwrap();
    // serde_json serializes MessageType::Text as the quoted string "text"
    assert_eq!(
        mt, "\"text\"",
        "message_type metadata should be serialized MessageType"
    );
}

#[tokio::test]
async fn test_message_type_image_injected() {
    use closeclaw_common::im_plugin::MessageType;

    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_chat".to_string(),
        content: String::new(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: MessageType::Image,
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let mt = metadata_str(&result.metadata, "message_type").unwrap();
    assert_eq!(
        mt, "\"image\"",
        "message_type metadata should be serialized MessageType::Image"
    );
}

#[tokio::test]
async fn test_message_type_file_injected() {
    use closeclaw_common::im_plugin::MessageType;

    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_chat".to_string(),
        content: "check this file".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: MessageType::File,
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let mt = metadata_str(&result.metadata, "message_type").unwrap();
    assert_eq!(
        mt, "\"file\"",
        "message_type metadata should be serialized MessageType::File"
    );
}

#[tokio::test]
async fn test_message_type_audio_injected() {
    use closeclaw_common::im_plugin::MessageType;

    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_chat".to_string(),
        content: String::new(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: MessageType::Audio,
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let mt = metadata_str(&result.metadata, "message_type").unwrap();
    assert_eq!(
        mt, "\"audio\"",
        "message_type metadata should be serialized MessageType::Audio"
    );
}

#[tokio::test]
async fn test_message_type_post_injected() {
    use closeclaw_common::im_plugin::MessageType;

    let router = make_router();
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user".to_string(),
        peer_id: "oc_chat".to_string(),
        content: "rich post content".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: MessageType::Post,
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx(msg);
    let result = router.process(&ctx).await.unwrap().unwrap();
    let mt = metadata_str(&result.metadata, "message_type").unwrap();
    assert_eq!(
        mt, "\"post\"",
        "message_type metadata should be serialized MessageType::Post"
    );
}
