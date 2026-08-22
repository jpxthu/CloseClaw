//! Tests for streaming finish-phase chain and incremental-phase chain.
//!
//! **Finish-phase tests** verify the outbound processor chain applied
//! during the streaming finish phase (post-stream pipeline):
//! VerbosityFilter → DslParser → OutboundRawLog in order.
//!
//! **Incremental-phase tests** verify `process_outbound_incremental`:
//! runs VerbosityFilter only, skipping DslParser and OutboundRawLog
//! (DslParser is a zero-overhead passthrough per the design doc).

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_llm::types::ContentBlock;

use super::dsl_parser::DslParser;
use super::registry::ProcessorRegistry;
use super::verbosity_filter::VerbosityFilter;
use super::ProcessedMessage;

// ── helpers ──────────────────────────────────────────────────────────────────

fn thinking_block(s: &str) -> ContentBlock {
    ContentBlock::Thinking {
        thinking: s.to_string(),
        signature: None,
    }
}

fn text_block(s: &str) -> ContentBlock {
    ContentBlock::Text(s.to_string())
}

fn image_block(name: &str) -> ContentBlock {
    ContentBlock::Image {
        name: name.to_string(),
        url: format!("https://example.com/{name}"),
    }
}

fn audio_block(name: &str) -> ContentBlock {
    ContentBlock::Audio {
        name: name.to_string(),
        url: format!("https://example.com/{name}"),
    }
}

fn file_block(name: &str) -> ContentBlock {
    ContentBlock::File {
        name: name.to_string(),
        url: format!("https://example.com/{name}"),
    }
}

fn make_meta(verbosity: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("verbosity_level".to_string(), verbosity.to_string());
    m
}

/// Build the full outbound chain: VerbosityFilter → DslParser.
fn build_full_chain() -> ProcessorRegistry {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    registry
}

// ═══════════════════════════════════════════════════════════════════════════════
// Finish-phase chain: VerbosityFilter → DslParser execution order
// ═══════════════════════════════════════════════════════════════════════════════

/// Streaming finish phase: Off verbosity + DSL in content.
/// VerbosityFilter runs first (removes non-Text/media), then DslParser processes.
#[tokio::test]
async fn test_finish_phase_off_filters_before_dsl_parse() {
    let registry = build_full_chain();
    let blocks = vec![
        thinking_block("internal reasoning"),
        text_block("::button[label:OK;action:submit]"),
        text_block("Plain text"),
    ];
    let output = ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("off"),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Off: Thinking filtered, two Text blocks remain
    // DslParser: first Text (DSL-only) stripped, second Text kept
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Text(s) if s == "Plain text"
    ));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

/// Streaming finish phase: Full verbosity, no DSL — all blocks pass through.
#[tokio::test]
async fn test_finish_phase_full_preserves_all_blocks() {
    let registry = build_full_chain();
    let blocks = vec![
        thinking_block("reasoning"),
        text_block("Hello"),
        image_block("photo.png"),
    ];
    let output = ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("full"),
    };
    let result = registry.process_outbound(output).await.unwrap();

    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(&result.content_blocks[1], ContentBlock::Text(s) if s == "Hello"));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::Image { .. }
    ));
}

/// Streaming finish phase: Off verbosity with media blocks.
/// Media blocks are always shown regardless of verbosity level.
#[tokio::test]
async fn test_finish_phase_off_keeps_media_blocks() {
    let registry = build_full_chain();
    let blocks = vec![
        thinking_block("hidden"),
        text_block("response"),
        image_block("chart.png"),
        audio_block("voice.wav"),
        file_block("report.pdf"),
    ];
    let output = ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("off"),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Off: keep Text + Image + Audio + File, filter Thinking
    assert_eq!(result.content_blocks.len(), 4);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "response"));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Image { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::Audio { .. }
    ));
    assert!(matches!(
        &result.content_blocks[3],
        ContentBlock::File { .. }
    ));
}

/// Streaming finish phase: Normal verbosity filters Thinking, preserves ToolUse.
#[tokio::test]
async fn test_finish_phase_normal_filters_thinking_preserves_tools() {
    let registry = build_full_chain();
    let blocks = vec![
        thinking_block("step 1"),
        text_block("result"),
        ContentBlock::ToolUse {
            id: "c1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "c1".into(),
            content: "ok".into(),
        },
    ];
    let output = ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("normal"),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Normal: Thinking filtered, 3 blocks remain
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "result"));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::ToolUse { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::ToolResult { .. }
    ));
}

/// Streaming finish phase: empty blocks → VerbosityFilter wraps content as Text.
#[tokio::test]
async fn test_finish_phase_empty_blocks_wraps_content() {
    let registry = build_full_chain();
    let output = ProcessedMessage {
        content_blocks: vec![],
        metadata: make_meta("normal"),
    };
    let result = registry.process_outbound(output).await.unwrap();

    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Incremental-phase tests (process_outbound_incremental)
// ═══════════════════════════════════════════════════════════════════════════════

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use crate::processor_chain::context::MessageContext;
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use closeclaw_common::ProcessorChain;

/// A test processor that counts invocations and optionally injects metadata.
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

/// Incremental phase: only VerbosityFilter runs; DslParser and OutboundRawLog
/// are both skipped (DslParser is a zero-overhead passthrough per design doc).
#[tokio::test]
async fn test_incremental_skips_dsl_parser_and_raw_log() {
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
        name: "DslParser".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 10,
        call_counter: dsl_counter.clone(),
        metadata_kv: None,
    });
    let raw_log_counter = Arc::new(AtomicUsize::new(0));
    let raw_log = Arc::new(TestProc {
        name: "outbound_raw_log".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 20,
        call_counter: raw_log_counter.clone(),
        metadata_kv: None,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(verbosity);
    registry.register(dsl);
    registry.register(raw_log);

    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("test".to_string())],
        metadata: HashMap::new(),
    };
    let result = registry.process_outbound_incremental(msg).await.unwrap();

    assert_eq!(
        vf_counter.load(Ordering::SeqCst),
        1,
        "verbosity_filter should run"
    );
    assert_eq!(
        dsl_counter.load(Ordering::SeqCst),
        0,
        "dsl_parser must be skipped (zero-overhead passthrough in incremental phase)"
    );
    assert_eq!(
        raw_log_counter.load(Ordering::SeqCst),
        0,
        "outbound_raw_log must be skipped"
    );
    // Output reflects last executed processor (verbosity_filter)
    assert_eq!(result.text_content(), Some("verbosity_filter"));
}

/// Incremental phase: VerbosityFilter filters Thinking blocks.
///
/// Input contains Thinking + Text blocks at "off" verbosity.
/// VerbosityFilter should remove the Thinking block.
#[tokio::test]
async fn test_incremental_verbosity_filter_works() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));

    let blocks = vec![
        thinking_block("internal reasoning"),
        text_block("visible text"),
    ];
    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("off"),
    };
    let result = registry.process_outbound_incremental(msg).await.unwrap();

    // Off verbosity: Thinking filtered, only Text remains
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "visible text"));
}

/// Incremental phase: DslParser is a passthrough, DSL lines are not stripped.
///
/// Input contains a DSL line + plain text. DslParser is skipped in the
/// incremental phase, so both blocks pass through unchanged.
#[tokio::test]
async fn test_incremental_dsl_parser_passthrough() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));

    let blocks = vec![
        text_block("::button[label:OK;action:submit]"),
        text_block("Hello world"),
    ];
    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("full"),
    };
    let result = registry.process_outbound_incremental(msg).await.unwrap();

    // DslParser skipped — both text blocks pass through unchanged
    assert_eq!(result.content_blocks.len(), 2);
    assert!(
        matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "::button[label:OK;action:submit]")
    );
    assert!(matches!(&result.content_blocks[1], ContentBlock::Text(s) if s == "Hello world"));
    // No DSL result in metadata since DslParser did not run
    assert!(result.metadata.get("dsl_result").is_none());
}

/// Default trait implementation: non-registry impl delegates to full outbound chain.
///
/// Verifies backward compatibility: a bare ProcessorChain impl's
/// `process_outbound_incremental` falls back to `process_outbound`.
#[tokio::test]
async fn test_default_impl_delegates_to_full_chain() {
    struct DummyChain;

    #[async_trait]
    impl ProcessorChain for DummyChain {
        async fn process_inbound(
            &self,
            _msg: closeclaw_common::im_plugin::NormalizedMessage,
        ) -> Result<
            closeclaw_common::processor::ProcessedMessage,
            closeclaw_common::processor::ProcessError,
        > {
            unimplemented!()
        }

        async fn process_outbound(
            &self,
            msg: closeclaw_common::processor::ProcessedMessage,
        ) -> Result<
            closeclaw_common::processor::ProcessedMessage,
            closeclaw_common::processor::ProcessError,
        > {
            // Signal that full chain ran by injecting metadata
            let mut m = msg;
            m.metadata
                .insert("full_chain".to_string(), "yes".to_string());
            Ok(m)
        }
    }

    let chain = DummyChain;
    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("test".to_string())],
        metadata: HashMap::new(),
    };
    let result = chain.process_outbound_incremental(msg).await.unwrap();

    assert_eq!(
        result.metadata.get("full_chain").map(|s| s.as_str()),
        Some("yes"),
        "default impl must delegate to process_outbound"
    );
}
