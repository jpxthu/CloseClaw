//! Tests for `process_outbound_without_verbosity` (Step 1.1).
//!
//! Verifies that the streaming finish phase skips VerbosityFilter while
//! still running other outbound processors (DslParser, OutboundRawLog).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::processor_chain::context::MessageContext;
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use crate::processor_chain::registry::ProcessorRegistry;
use crate::ProcessedMessage;
use closeclaw_common::ProcessorChain;
use closeclaw_llm::types::ContentBlock;

// ── helpers ──────────────────────────────────────────────────────────────────

struct TestProc {
    name: String,
    phase: ProcessPhase,
    priority: u8,
    call_counter: Arc<AtomicUsize>,
    metadata_kv: Option<(String, String)>,
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

// ── tests ────────────────────────────────────────────────────────────────────

/// `process_outbound_without_verbosity` skips VerbosityFilter while running
/// other outbound processors (DslParser, OutboundRawLog).
#[tokio::test]
async fn test_skips_verbosity_filter() {
    let vf_counter = Arc::new(AtomicUsize::new(0));
    let verbosity = Arc::new(TestProc {
        name: "verbosity_filter".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 5,
        call_counter: vf_counter.clone(),
        metadata_kv: None,
    });
    let dsl_counter = Arc::new(AtomicUsize::new(0));
    let dsl = Arc::new(TestProc {
        name: "dsl_parser".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 10,
        call_counter: dsl_counter.clone(),
        metadata_kv: None,
    });
    let raw_log_counter = Arc::new(AtomicUsize::new(0));
    let raw_log = Arc::new(TestProc {
        name: "outbound_raw_log".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 15,
        call_counter: raw_log_counter.clone(),
        metadata_kv: None,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(verbosity);
    registry.register(dsl);
    registry.register(raw_log);

    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("test output".to_string())],
        metadata: HashMap::new(),
    };
    let result: closeclaw_common::processor::ProcessedMessage = registry
        .process_outbound_without_verbosity(msg)
        .await
        .unwrap();

    // VerbosityFilter should NOT have been called
    assert_eq!(
        vf_counter.load(Ordering::SeqCst),
        0,
        "verbosity_filter must be skipped"
    );
    // DslParser and OutboundRawLog should have been called
    assert_eq!(
        dsl_counter.load(Ordering::SeqCst),
        1,
        "dsl_parser should run"
    );
    assert_eq!(
        raw_log_counter.load(Ordering::SeqCst),
        1,
        "outbound_raw_log should run"
    );
    // Output reflects last processor in chain (raw_log)
    assert_eq!(result.text_content(), Some("outbound_raw_log"));
}

/// `process_outbound_without_verbosity` with empty chain returns input unchanged.
#[tokio::test]
async fn test_empty_chain_passthrough() {
    let registry = ProcessorRegistry::new();
    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("passthrough".to_string())],
        metadata: HashMap::new(),
    };
    let result: closeclaw_common::processor::ProcessedMessage = registry
        .process_outbound_without_verbosity(msg)
        .await
        .unwrap();
    assert_eq!(result.text_content(), Some("passthrough"));
}

/// `process_outbound_without_verbosity` preserves metadata from non-excluded processors.
#[tokio::test]
async fn test_preserves_metadata_from_non_excluded() {
    let verbosity = Arc::new(TestProc {
        name: "verbosity_filter".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 5,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("skip_me".to_string(), "yes".to_string())),
    });
    let dsl = Arc::new(TestProc {
        name: "dsl_parser".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 10,
        call_counter: Arc::new(AtomicUsize::new(0)),
        metadata_kv: Some(("dsl_key".to_string(), "dsl_val".to_string())),
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(verbosity);
    registry.register(dsl);

    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("input".to_string())],
        metadata: HashMap::new(),
    };
    let result: closeclaw_common::processor::ProcessedMessage = registry
        .process_outbound_without_verbosity(msg)
        .await
        .unwrap();

    assert_eq!(result.text_content(), Some("dsl_parser"));
    assert_eq!(
        result.metadata.get("dsl_key").map(|s| s.as_str()),
        Some("dsl_val")
    );
    assert!(
        result.metadata.get("skip_me").is_none(),
        "VerbosityFilter metadata should not appear"
    );
}
