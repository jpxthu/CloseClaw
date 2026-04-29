//! Tests for [`ProcessorRegistry`] — kept in a separate file to respect the
//! ≤ 500-line limit per source file.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::processor_chain::context::{MessageContext, ProcessedMessage};
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use crate::processor_chain::registry::ProcessorRegistry;
use crate::processor_chain::RawMessage;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_raw(content: &str) -> RawMessage {
    RawMessage {
        platform: "test".to_string(),
        sender_id: "user_1".to_string(),
        content: content.to_string(),
        timestamp: Utc::now(),
        message_id: "msg_1".to_string(),
    }
}

/// Test processor that sets its own name as content and records call count.
struct TestProc {
    name: String,
    phase: ProcessPhase,
    priority: u8,
    call_counter: Arc<AtomicUsize>,
    metadata_kv: Option<(String, serde_json::Value)>,
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

    fn with_metadata(mut self, key: &str, val: serde_json::Value) -> Self {
        self.metadata_kv = Some((key.to_string(), val));
        self
    }
    fn with_suppress(mut self) -> Self {
        self.suppress = true;
        self
    }
    fn with_skip(mut self) -> Self {
        self.skip = true;
        self
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
        if self.skip {
            return Ok(None);
        }
        let mut metadata = serde_json::Map::new();
        if let Some((ref k, ref v)) = self.metadata_kv {
            metadata.insert(k.clone(), v.clone());
        }
        Ok(Some(ProcessedMessage {
            content: self.name.clone(),
            metadata,
            suppress: self.suppress,
        }))
    }
}

// ── inbound bypass ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_inbound_bypass() {
    let registry = ProcessorRegistry::new();
    let raw = make_raw("hello world");
    let result = registry.process_inbound(raw.clone()).await.unwrap();

    assert_eq!(result.content, raw.content);
    assert!(!result.suppress);
    assert_eq!(result.metadata.len(), 0);
}

// ── outbound bypass ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_outbound_bypass() {
    let registry = ProcessorRegistry::new();
    let llm_out = ProcessedMessage {
        content: "llm said hello".to_string(),
        metadata: serde_json::Map::new(),
        suppress: false,
    };
    let result = registry.process_outbound(llm_out.clone()).await.unwrap();

    assert_eq!(result.content, llm_out.content);
    assert!(!result.suppress);
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

    let result = registry.process_inbound(make_raw("orig")).await.unwrap();

    assert_eq!(c5.load(Ordering::SeqCst), 1, "p_5 should be called once");
    assert_eq!(c10.load(Ordering::SeqCst), 1, "p_10 should be called once");
    assert_eq!(result.content, "p_20");
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
        .process_inbound(make_raw("original"))
        .await
        .unwrap();

    assert_eq!(result.content, "p3");
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
            content: "llm".to_string(),
            metadata: serde_json::Map::new(),
            suppress: false,
        })
        .await
        .unwrap();

    assert_eq!(result.content, "o2");
}

// ── error propagation ────────────────────────────────────────────────────────

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
async fn test_inbound_error_propagates() {
    let failing = FailingProc::inbound("fail");
    let mut registry = ProcessorRegistry::new();
    registry.register(failing);

    let err = registry
        .process_inbound(make_raw("hello"))
        .await
        .unwrap_err();
    match err {
        ProcessError::ProcessorFailed { name, .. } => assert_eq!(name, "fail"),
        _ => panic!("expected ProcessorFailed, got {:?}", err),
    }
}

#[tokio::test]
async fn test_outbound_error_propagates() {
    let failing = FailingProc::outbound("fail_out");
    let mut registry = ProcessorRegistry::new();
    registry.register(failing);

    let err = registry
        .process_outbound(ProcessedMessage {
            content: "llm".to_string(),
            metadata: serde_json::Map::new(),
            suppress: false,
        })
        .await
        .unwrap_err();

    match err {
        ProcessError::ProcessorFailed { name, .. } => assert_eq!(name, "fail_out"),
        _ => panic!("expected ProcessorFailed, got {:?}", err),
    }
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

    registry.process_inbound(make_raw("hello")).await.unwrap();

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

    let result = registry.process_inbound(make_raw("hello")).await.unwrap();

    assert!(result.suppress);
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
        metadata_kv: Some(("key2".to_string(), serde_json::json!("value2"))),
        suppress: false,
        skip: false,
    });
    let p3 = Arc::new(TestProc {
        name: "p3".to_string(),
        phase: ProcessPhase::Inbound,
        priority: 3,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("key3".to_string(), serde_json::json!("value3"))),
        suppress: false,
        skip: false,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(p1);
    registry.register(p2);
    registry.register(p3);

    let result = registry.process_inbound(make_raw("orig")).await.unwrap();

    assert_eq!(result.content, "p3");
    assert_eq!(
        result.metadata.get("key2").and_then(|v| v.as_str()),
        Some("value2")
    );
    assert_eq!(
        result.metadata.get("key3").and_then(|v| v.as_str()),
        Some("value3")
    );
}
