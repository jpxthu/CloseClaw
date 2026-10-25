//! Tests for [`ProcessorRegistry`] — kept in a separate file to respect the
//! 1000-line limit per source file.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::processor_chain::context::metadata_str;
use crate::processor_chain::processor::ProcessPhase;
use crate::processor_chain::registry::ProcessorRegistry;
use crate::ProcessedMessage;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_common::processor::ContentBlock;

use crate::content_normalizer::ContentNormalizer;
use crate::processor_chain::test_support::{make_normalized, TestProc};

// ── inbound bypass ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_inbound_bypass() {
    let registry = ProcessorRegistry::new();
    let msg = make_normalized("hello world");
    let result = registry.process_inbound(msg).await.unwrap();
    assert_eq!(result.text_content(), Some("hello world"));
    assert!(!result.content_blocks.is_empty());
    // Design doc: chain dispatcher copies message_type + unavailable_media
    assert_eq!(result.metadata.len(), 2);
    assert!(result.metadata.contains_key("message_type"));
    assert!(result.metadata.contains_key("unavailable_media"));
    assert_eq!(result.metadata.get("message_type").unwrap(), "\"text\"");
    assert_eq!(result.metadata.get("unavailable_media").unwrap(), "[]");
}

// ── outbound bypass ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_outbound_bypass() {
    let registry = ProcessorRegistry::new();
    let llm_out = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("llm said hello".to_string())],
        metadata: HashMap::new(),
    };
    let result = registry.process_outbound(llm_out.clone()).await.unwrap();

    assert_eq!(result.text_content(), Some("llm said hello"));
    assert!(!result.content_blocks.is_empty());
    assert_eq!(result.metadata.len(), 0);
}

// ── priority ascending ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_inbound_priority_ascending() {
    let (p_10, c10) = TestProc::inbound("p_10", 10);
    let (p_5, c5) = TestProc::inbound("p_5", 5);
    let (p_20, _) = TestProc::inbound("p_20", 20);
    let mut registry = ProcessorRegistry::new();
    registry.register(p_10.clone());
    registry.register(p_5.clone());
    registry.register(p_20.clone());

    let result = registry
        .process_inbound(make_normalized("orig"))
        .await
        .unwrap();

    assert_eq!(c5.load(Ordering::SeqCst), 1, "p_5 should be called once");
    assert_eq!(c10.load(Ordering::SeqCst), 1, "p_10 should be called once");
    assert_eq!(result.text_content(), Some("p_20"));
}

// ── chained processors: latter overrides former ───────────────────────────────

#[tokio::test]
async fn test_inbound_chained_override() {
    let (p1, _) = TestProc::inbound("p1", 1);
    let (p2, _) = TestProc::inbound("p2", 2);
    let (p3, _) = TestProc::inbound("p3", 3);

    let mut registry = ProcessorRegistry::new();
    registry.register(p1);
    registry.register(p2);
    registry.register(p3);

    let result = registry
        .process_inbound(make_normalized("original"))
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("p3"));
}

#[tokio::test]
async fn test_outbound_chained_override() {
    let (o1, _) = TestProc::outbound("o1", 1);
    let (o2, _) = TestProc::outbound("o2", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(o1);
    registry.register(o2);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("llm".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("o2"));
}

// ── skip halts chain ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_skip_halts_inbound_chain() {
    let (p1, c1) = TestProc::inbound("p1", 1);
    let skip_counter = Arc::new(AtomicUsize::new(0));
    let p2_skip = Arc::new(TestProc {
        name: "p2_skip".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 2,
        call_counter: skip_counter,
        metadata_kv: None,
        suppress: false,
        skip: true,
    });
    let (p3, c3) = TestProc::inbound("p3", 3);

    let mut registry = ProcessorRegistry::new();
    registry.register(p1.clone());
    registry.register(p2_skip);
    registry.register(p3.clone());

    registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    assert_eq!(c1.load(Ordering::SeqCst), 1, "p1 should be called");
    assert_eq!(c3.load(Ordering::SeqCst), 0, "p3 should NOT run after skip");
}

// ── suppress halts chain ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_suppress_halts_inbound_chain() {
    let (p1, c1) = TestProc::inbound("p1", 1);
    let suppress_counter = Arc::new(AtomicUsize::new(0));
    let p2_suppress = Arc::new(TestProc {
        name: "p2_suppress".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 2,
        call_counter: suppress_counter,
        metadata_kv: None,
        suppress: true,
        skip: false,
    });
    let (p3, c3) = TestProc::inbound("p3", 3);

    let mut registry = ProcessorRegistry::new();
    registry.register(p1.clone());
    registry.register(p2_suppress);
    registry.register(p3.clone());

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    assert!(result.content_blocks.is_empty());
    assert_eq!(c1.load(Ordering::SeqCst), 1, "p1 should be called");
    assert_eq!(
        c3.load(Ordering::SeqCst),
        0,
        "p3 should NOT run after suppress"
    );
}

// ── metadata merging ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_metadata_merged_across_chain() {
    let (p1, _) = TestProc::inbound("p1", 1);
    let p2 = Arc::new(TestProc {
        name: "p2".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 2,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("key2".to_string(), "value2".to_string())),
        suppress: false,
        skip: false,
    });
    let p3 = Arc::new(TestProc {
        name: "p3".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 3,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("key3".to_string(), "value3".to_string())),
        suppress: false,
        skip: false,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(p1);
    registry.register(p2);
    registry.register(p3);

    let result = registry
        .process_inbound(make_normalized("orig"))
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("p3"));
    assert_eq!(metadata_str(&result.metadata, "key2"), Some("value2"));
    assert_eq!(metadata_str(&result.metadata, "key3"), Some("value3"));
}

// ── NormalizedMessage → ProcessorChain end-to-end inbound ────────────────────
// Verifies that NormalizedMessage is passed directly into ProcessorChain
// (no RawMessage intermediary) and that the inbound chain produces the
// expected ProcessedMessage output.

#[tokio::test]
async fn test_normalized_message_directly_into_processor_chain() {
    // Build a registry with a single test processor.
    let (proc, counter) = TestProc::inbound("normalizer", 10);
    let mut registry = ProcessorRegistry::new();
    registry.register(proc);

    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_user1".to_string(),
        peer_id: "oc_chat1".to_string(),
        content: "hello from normalized".to_string(),
        timestamp: 1700000000000,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: "acct_1".to_string(),
        ..Default::default()
    };

    let result = registry.process_inbound(msg).await.unwrap();

    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("normalizer"));
}

#[tokio::test]
async fn test_normalized_message_empty_content() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));

    let msg = NormalizedMessage {
        platform: "terminal".to_string(),
        sender_id: "owner".to_string(),
        peer_id: String::new(),
        content: String::new(),
        timestamp: 0,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: "owner".to_string(),
        ..Default::default()
    };

    let result = registry.process_inbound(msg).await.unwrap();
    // Empty content stays empty after normalization.
    assert_eq!(result.text_content(), Some(""));
    assert!(!result.content_blocks.is_empty());
}

#[tokio::test]
async fn test_normalized_message_with_special_characters() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(ContentNormalizer::new()));

    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "ou_1".to_string(),
        peer_id: "oc_1".to_string(),
        content: "hello\x1b[31mworld\x1b[0m\r\nextra  ".to_string(),
        timestamp: 1700000000000,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: String::new(),
        ..Default::default()
    };

    let result = registry.process_inbound(msg).await.unwrap();
    // ANSI stripped, trailing whitespace trimmed, \r removed.
    let text = result.text_content().unwrap();
    assert!(!text.contains("\x1b"));
    assert!(
        !text.ends_with(' '),
        "trailing whitespace should be trimmed"
    );
}

#[tokio::test]
async fn test_normalized_message_timestamp_is_i64_millis() {
    let msg = NormalizedMessage {
        platform: "test".to_string(),
        sender_id: "s".to_string(),
        peer_id: "p".to_string(),
        content: "x".to_string(),
        timestamp: 1700000000123,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: String::new(),
        ..Default::default()
    };
    // Verify timestamp is i64 milliseconds (not DateTime<Utc>).
    assert!(msg.timestamp > 1_000_000_000_000);
}

#[tokio::test]
async fn test_normalized_message_passthrough_no_processors() {
    // When no processors are registered, NormalizedMessage content
    // is wrapped directly into a ProcessedMessage.
    let registry = ProcessorRegistry::new();
    let msg = NormalizedMessage {
        platform: "test".to_string(),
        sender_id: "s".to_string(),
        peer_id: "p".to_string(),
        content: "direct passthrough".to_string(),
        timestamp: 0,
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: String::new(),
        ..Default::default()
    };

    let result = registry.process_inbound(msg).await.unwrap();
    assert_eq!(result.text_content(), Some("direct passthrough"));
    // Design doc: chain dispatcher copies message_type + unavailable_media
    // even for empty chain bypass.
    assert!(result.metadata.contains_key("message_type"));
    assert!(result.metadata.contains_key("unavailable_media"));
    assert_eq!(result.metadata.get("message_type").unwrap(), "\"text\"");
    assert_eq!(result.metadata.get("unavailable_media").unwrap(), "[]");
}
