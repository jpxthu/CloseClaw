//! Tests for [`ProcessorRegistry`] outbound processor fallback — a failing
//! outbound processor must not halt the chain (fail-open): the context stays
//! unchanged and downstream processors still run.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::processor_chain::context::MessageContext;
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use crate::processor_chain::registry::ProcessorRegistry;
use crate::ProcessedMessage;
use closeclaw_common::processor::ContentBlock;

use crate::processor_chain::test_support::FailingProc;

// ── Processor error fallback ─────────────────────────────────────────────────
// Design doc: VerbosityFilter fail → Full behavior, DslParser fail → passthrough,
// OutboundRawLog fail → skip log, continue.

/// Pass-through processor: echoes back whatever content_blocks it received.
struct PassThroughProc {
    name: String,
    phase: ProcessPhase,
    priority: u8,
    call_counter: Arc<AtomicUsize>,
}

#[async_trait]
impl MessageProcessor for PassThroughProc {
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
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        self.call_counter.fetch_add(1, Ordering::SeqCst);
        Ok(Some(ProcessedMessage {
            content_blocks: ctx.content_blocks.clone(),
            metadata: ctx.metadata.clone(),
        }))
    }
}

/// VerbosityFilter failure on outbound: ctx is not updated, so subsequent
/// processors see the original content_blocks (Full behavior).
#[tokio::test]
async fn test_outbound_verbosity_filter_fail_keeps_original_blocks() {
    let fail_vf = FailingProc::outbound("verbosity_filter");
    let trace_counter = Arc::new(AtomicUsize::new(0));
    let trace = Arc::new(PassThroughProc {
        name: "trace_after_vf".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 20,
        call_counter: trace_counter.clone(),
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_vf);
    registry.register(trace);

    let blocks = vec![
        ContentBlock::Text("hello".to_string()),
        ContentBlock::Thinking {
            thinking: "hidden".to_string(),
            signature: None,
        },
    ];
    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: blocks,
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    // fail_vf failed → ctx unchanged → trace sees original 2 blocks (Full behavior)
    assert_eq!(trace_counter.load(Ordering::SeqCst), 1);
    assert_eq!(
        result.content_blocks.len(),
        2,
        "should keep all blocks (Full)"
    );
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "hello"));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Thinking { .. }
    ));
}

/// DslParser failure on outbound: ctx is not updated, so original Text
/// blocks with DSL lines are preserved (passthrough).
#[tokio::test]
async fn test_outbound_dsl_parser_fail_preserves_dsl_lines() {
    let fail_dsl = FailingProc::outbound("dsl_parser");
    let trace_counter = Arc::new(AtomicUsize::new(0));
    let trace = Arc::new(PassThroughProc {
        name: "trace_after_dsl".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 20,
        call_counter: trace_counter.clone(),
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_dsl);
    registry.register(trace);

    let blocks = vec![ContentBlock::Text(
        "::button[label:OK;action:submit]".to_string(),
    )];
    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: blocks,
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    // fail_dsl failed → ctx unchanged → trace sees original block with DSL line
    assert_eq!(trace_counter.load(Ordering::SeqCst), 1);
    assert_eq!(result.content_blocks.len(), 1);
    assert!(
        matches!(&result.content_blocks[0], ContentBlock::Text(s) if s.contains("button")),
        "DSL line should be preserved in passthrough"
    );
}

/// OutboundRawLog failure: ctx unchanged, continues chain.
#[tokio::test]
async fn test_outbound_raw_log_fail_continues_chain() {
    let fail_raw = FailingProc::outbound("outbound_raw_log");
    let trace_counter = Arc::new(AtomicUsize::new(0));
    let trace = Arc::new(PassThroughProc {
        name: "after_raw_log".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 30,
        call_counter: trace_counter.clone(),
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(fail_raw);
    registry.register(trace);

    let result = registry
        .process_outbound(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text("test".to_string())],
            metadata: HashMap::new(),
        })
        .await
        .unwrap();

    assert_eq!(trace_counter.load(Ordering::SeqCst), 1);
    assert_eq!(
        result.text_content(),
        Some("test"),
        "original content preserved through fail-open"
    );
}
