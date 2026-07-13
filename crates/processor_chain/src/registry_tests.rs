//! Tests for [`ProcessorRegistry`] — kept in a separate file to respect the
//! ≤ 500-line limit per source file.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tracing_test::traced_test;

use crate::processor_chain::context::MessageContext;
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use crate::processor_chain::registry::ProcessorRegistry;
use crate::ProcessedMessage;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_llm::types::ContentBlock;

use crate::content_normalizer::ContentNormalizer;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_normalized(content: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "test".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: String::new(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: String::new(),
    }
}

/// Test processor that sets its own name as content and records call count.
struct TestProc {
    name: String,
    phase: ProcessPhase,
    priority: u8,
    call_counter: Arc<AtomicUsize>,
    metadata_kv: Option<(String, String)>,
    suppress: bool,
    skip: bool,
}

impl TestProc {
    fn inbound(name: &str, priority: u8) -> (Arc<Self>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let proc = Self {
            name: name.to_string(),
            phase: ProcessPhase::Inbound,
            priority,
            call_counter: counter.clone(),
            metadata_kv: None,
            suppress: false,
            skip: false,
        };
        (Arc::new(proc), counter)
    }

    fn outbound(name: &str, priority: u8) -> (Arc<Self>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let proc = Self {
            name: name.to_string(),
            phase: ProcessPhase::Outbound,
            priority,
            call_counter: counter.clone(),
            metadata_kv: None,
            suppress: false,
            skip: false,
        };
        (Arc::new(proc), counter)
    }
}

#[async_trait]
impl MessageProcessor for TestProc {
    fn name(&self) -> &str {
        &self.name
    }
    fn phase(&self) -> ProcessPhase {
        self.phase
    }
    fn priority(&self) -> u8 {
        self.priority
    }

    async fn process(
        &self,
        _ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        self.call_counter.fetch_add(1, Ordering::SeqCst);
        if self.skip || self.suppress {
            return Ok(None);
        }
        let mut metadata = HashMap::new();
        if let Some((ref k, ref v)) = self.metadata_kv {
            metadata.insert(k.clone(), v.clone());
        }
        Ok(Some(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(self.name.clone())],
            metadata,
        }))
    }
}

// ── inbound bypass ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_inbound_bypass() {
    let registry = ProcessorRegistry::new();
    let msg = make_normalized("hello world");
    let result = registry.process_inbound(msg).await.unwrap();

    assert_eq!(result.text_content(), Some("hello world"));
    assert!(!result.content_blocks.is_empty());
    assert_eq!(result.metadata.len(), 0);
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

// ── error tolerance ──────────────────────────────────────────────────────────
// A failing processor should not halt the chain; subsequent processors still run.

struct FailingProc {
    name: String,
    phase: ProcessPhase,
    priority: u8,
}

impl FailingProc {
    fn inbound(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            phase: ProcessPhase::Inbound,
            priority: 1,
        })
    }
    fn outbound(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            phase: ProcessPhase::Outbound,
            priority: 1,
        })
    }
}

#[async_trait]
impl MessageProcessor for FailingProc {
    fn name(&self) -> &str {
        &self.name
    }
    fn phase(&self) -> ProcessPhase {
        self.phase
    }
    fn priority(&self) -> u8 {
        self.priority
    }

    async fn process(&self, _: &MessageContext) -> Result<Option<ProcessedMessage>, ProcessError> {
        Err(ProcessError::processor_failed(
            self.name(),
            "intentional failure",
        ))
    }
}

#[tokio::test]
async fn test_inbound_error_continues_chain() {
    let (ok_proc, ok_counter) = TestProc::inbound("ok_before", 1);
    let fail_proc = FailingProc::inbound("fail_mid");
    let (ok_after, ok_after_counter) = TestProc::inbound("ok_after", 3);

    let mut registry = ProcessorRegistry::new();
    registry.register(ok_proc);
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    // ok_before runs and updates ctx, fail_mid fails (ctx unchanged), ok_after runs
    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(ok_after_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
}

#[tokio::test]
async fn test_inbound_first_processor_fails() {
    let fail_proc = FailingProc::inbound("fail_first");
    let (ok_after, ok_counter) = TestProc::inbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("original"))
        .await
        .unwrap();

    // fail_first fails (ctx unchanged, original content preserved), ok_after runs
    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
}

#[tokio::test]
async fn test_inbound_only_processor_fails() {
    let fail_proc = FailingProc::inbound("solo_fail");

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);

    let result = registry
        .process_inbound(make_normalized("raw content"))
        .await
        .unwrap();

    // Single processor fails — ctx unchanged, raw content returned.
    assert_eq!(result.text_content(), Some("raw content"));
}

#[tokio::test]
async fn test_inbound_error_preserves_successful_metadata() {
    let p1 = Arc::new(TestProc {
        name: "p1".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 1,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k1".to_string(), "v1".to_string())),
        suppress: false,
        skip: false,
    });
    let fail_proc = FailingProc::inbound("p_fail");
    let p3 = Arc::new(TestProc {
        name: "p3".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 3,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k3".to_string(), "v3".to_string())),
        suppress: false,
        skip: false,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(p1);
    registry.register(fail_proc);
    registry.register(p3);

    let result = registry
        .process_inbound(make_normalized("orig"))
        .await
        .unwrap();

    // p1's metadata is preserved, p_fail failed (no metadata), p3's metadata added
    assert_eq!(result.text_content(), Some("p3"));
    assert_eq!(result.metadata.get("k1").map(|s| s.as_str()), Some("v1"));
    assert_eq!(result.metadata.get("k3").map(|s| s.as_str()), Some("v3"));
}

#[tokio::test]
async fn test_outbound_error_continues_chain() {
    let (ok_proc, ok_counter) = TestProc::outbound("ok_before", 1);
    let fail_proc = FailingProc::outbound("fail_mid");
    let (ok_after, ok_after_counter) = TestProc::outbound("ok_after", 3);

    let mut registry = ProcessorRegistry::new();
    registry.register(ok_proc);
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("llm".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(ok_after_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
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
    assert_eq!(
        result.metadata.get("key2").map(|s| s.as_str()),
        Some("value2")
    );
    assert_eq!(
        result.metadata.get("key3").map(|s| s.as_str()),
        Some("value3")
    );
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
    };

    let result = registry.process_inbound(msg).await.unwrap();
    assert_eq!(result.text_content(), Some("direct passthrough"));
    assert!(result.metadata.is_empty());
}

// ── outbound error tolerance: detailed tests ──────────────────────────────────

#[tokio::test]
async fn test_outbound_first_processor_fails() {
    let fail_proc = FailingProc::outbound("fail_first");
    let (ok_after, ok_counter) = TestProc::outbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);
    registry.register(ok_after);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("original".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("ok_after"));
}

#[tokio::test]
async fn test_outbound_only_processor_fails() {
    let fail_proc = FailingProc::outbound("solo_fail");

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_proc);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("raw content".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("raw content"));
}

#[tokio::test]
async fn test_outbound_error_preserves_successful_metadata() {
    let p1 = Arc::new(TestProc {
        name: "o1".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 1,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k1".to_string(), "v1".to_string())),
        suppress: false,
        skip: false,
    });
    let fail_proc = FailingProc::outbound("o_fail");
    let p3 = Arc::new(TestProc {
        name: "o3".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 3,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("k3".to_string(), "v3".to_string())),
        suppress: false,
        skip: false,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(p1);
    registry.register(fail_proc);
    registry.register(p3);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("orig".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("o3"));
    assert_eq!(result.metadata.get("k1").map(|s| s.as_str()), Some("v1"));
    assert_eq!(result.metadata.get("k3").map(|s| s.as_str()), Some("v3"));
}

// ── log level verification for fail-open handler ─────────────────────────────
// The fail-open handler uses `error!` for `raw_log` processor and
// `warn!` for all other processors. These tests use `tracing_test`
// to capture tracing output per-test and verify the correct log level.

#[tokio::test]
#[traced_test]
async fn test_raw_log_success_no_error_log() {
    let (raw_log, _) = TestProc::inbound("raw_log", 1);
    let (other, _) = TestProc::inbound("session_router", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(raw_log);
    registry.register(other);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();
    assert_eq!(result.text_content(), Some("session_router"));
    assert!(
        !logs_contain("ERROR"),
        "no error log expected when raw_log succeeds"
    );
}

#[tokio::test]
#[traced_test]
async fn test_raw_log_failure_produces_error_log() {
    let fail_raw = FailingProc::inbound("raw_log");
    let (ok_after, _) = TestProc::inbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_raw);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();
    assert_eq!(result.text_content(), Some("ok_after"));
    assert!(logs_contain("ERROR"), "expected ERROR for raw_log failure");
    assert!(!logs_contain("WARN"), "raw_log should use ERROR, not WARN");
}

#[tokio::test]
#[traced_test]
async fn test_non_raw_log_failure_produces_warn_log() {
    let fail_router = FailingProc::inbound("session_router");
    let (ok_after, _) = TestProc::inbound("ok_after", 2);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_router);
    registry.register(ok_after);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();
    assert_eq!(result.text_content(), Some("ok_after"));
    assert!(
        logs_contain("WARN"),
        "expected WARN for session_router failure"
    );
    assert!(
        !logs_contain("ERROR"),
        "non raw_log should use WARN, not ERROR"
    );
}

#[tokio::test]
#[traced_test]
async fn test_fail_open_continues_chain_regardless_of_processor() {
    // raw_log fails (error!), session_router succeeds,
    // content_normalizer fails (warn!), last processor runs.
    let fail_raw = FailingProc::inbound("raw_log");
    let (ok_mid, ok_counter) = TestProc::inbound("session_router", 2);
    let fail_norm = FailingProc::inbound("content_normalizer");
    let (ok_end, ok_end_counter) = TestProc::inbound("final_proc", 4);

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_raw);
    registry.register(ok_mid);
    registry.register(fail_norm);
    registry.register(ok_end);

    let result = registry
        .process_inbound(make_normalized("hello"))
        .await
        .unwrap();

    // All non-failing processors ran.
    assert_eq!(ok_counter.load(Ordering::SeqCst), 1);
    assert_eq!(ok_end_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.text_content(), Some("final_proc"));

    // raw_log failure → ERROR
    assert!(logs_contain("ERROR"), "expected ERROR for raw_log failure");
    // content_normalizer failure → WARN
    assert!(
        logs_contain("WARN"),
        "expected WARN for content_normalizer failure"
    );
}
