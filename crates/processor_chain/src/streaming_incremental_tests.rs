//! Tests for streaming finish-phase chain and incremental-phase chain.
//!
//! **Finish-phase tests** verify the outbound processor chain applied
//! during the streaming finish phase (post-stream pipeline):
//! VerbosityFilter → DslParser → OutboundRawLog in order.
//!
//! **Incremental-phase tests** verify `process_outbound_incremental`:
//! runs VerbosityFilter (normal mode) + DslParser (passthrough mode),
//! skipping OutboundRawLog.

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

fn tool_use_block(name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: "call_1".to_string(),
        name: name.to_string(),
        input: "{}".to_string(),
    }
}

fn tool_result_block(content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_call_id: "call_1".to_string(),
        content: content.to_string(),
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

/// Streaming finish phase: Off verbosity with mixed blocks.
/// Off level keeps only Text; filters Thinking, Image, Audio, File.
#[tokio::test]
async fn test_finish_phase_off_filters_media_blocks_keeps_text() {
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

    // Off: keep Text only, filter Thinking + Image + Audio + File
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "response"));
}

/// Streaming finish phase: Off verbosity with text-only input.
/// All text blocks should be preserved since Off keeps Text.
#[tokio::test]
async fn test_finish_phase_off_text_only_keeps_all() {
    let registry = build_full_chain();
    let blocks = vec![
        text_block("first"),
        text_block("second"),
        text_block("third"),
    ];
    let output = ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("off"),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Off: all Text blocks preserved
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "first"));
    assert!(matches!(&result.content_blocks[1], ContentBlock::Text(s) if s == "second"));
    assert!(matches!(&result.content_blocks[2], ContentBlock::Text(s) if s == "third"));
}

/// Streaming finish phase: Off verbosity with media-only input.
/// All media blocks (Image, Audio, File) and Thinking should be filtered.
/// Since no Text blocks remain, DslParser wraps content as a single Text block.
#[tokio::test]
async fn test_finish_phase_off_media_only_filters_all() {
    let registry = build_full_chain();
    let blocks = vec![
        thinking_block("internal"),
        image_block("photo.png"),
        audio_block("voice.wav"),
        file_block("report.pdf"),
    ];
    let output = ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("off"),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Off: no Text blocks in input → all filtered out
    // DslParser fallback: wraps content as Text when blocks are empty
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
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

/// Incremental phase: VerbosityFilter runs; DslParser is zero-overhead
/// passthrough (no parse); OutboundRawLog is skipped.
///
/// DslParser in incremental phase does NOT parse or write metadata.
/// Full parse runs in finalization phase.
/// Uses real VerbosityFilter + DslParser + mock OutboundRawLog.
#[tokio::test]
async fn test_incremental_runs_verbosity_and_dsl_passthrough() {
    let raw_log_counter = Arc::new(AtomicUsize::new(0));
    let raw_log = Arc::new(TestProc {
        name: "outbound_raw_log".to_string(),
        phase: ProcessPhase::Outbound,
        priority: 20,
        call_counter: raw_log_counter.clone(),
        metadata_kv: None,
    });

    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    registry.register(raw_log);

    let blocks = vec![
        text_block("::button[label:OK;action:submit]"),
        text_block("Hello world"),
    ];
    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: blocks,
        metadata: HashMap::new(),
    };
    let result = registry.process_outbound_incremental(msg).await.unwrap();

    // OutboundRawLog must be skipped.
    assert_eq!(
        raw_log_counter.load(Ordering::SeqCst),
        0,
        "outbound_raw_log must be skipped"
    );
    // DslParser zero-overhead: content blocks unchanged.
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Text(s)
            if s == "::button[label:OK;action:submit]"
    ));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Text(s) if s == "Hello world"
    ));
    // DslParser zero-overhead: no dsl_result in metadata.
    assert!(
        !result.metadata.contains_key("dsl_result"),
        "incremental phase must not write dsl_result (zero-overhead passthrough)"
    );
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

/// Incremental phase: DslParser is zero-overhead passthrough.
///
/// Input contains a DSL line + plain text. DslParser does NOT parse
/// or write metadata in incremental phase; content blocks pass through unchanged.
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

    // DslParser zero-overhead: content blocks unchanged
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Text(s)
            if s == "::button[label:OK;action:submit]"
    ));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Text(s) if s == "Hello world"
    ));
    // DslParser zero-overhead: no dsl_result in metadata
    assert!(
        !result.metadata.contains_key("dsl_result"),
        "incremental phase must not write dsl_result (zero-overhead passthrough)"
    );
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

/// Incremental phase with Off verbosity: VerbosityFilter runs first,
/// removing all non-Text blocks. DslParser is zero-overhead passthrough
/// (no parse, no metadata write); content blocks stay unchanged.
#[tokio::test]
async fn test_incremental_off_verbosity_filters_then_dsl_passthrough() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));

    let blocks = vec![
        thinking_block("internal reasoning"),
        text_block("::button[label:OK;action:submit]"),
        image_block("photo.png"),
        text_block("Plain response"),
    ];
    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("off"),
    };
    let result = registry.process_outbound_incremental(msg).await.unwrap();

    // Off: VerbosityFilter removes Thinking + Image, keeps 2 Text blocks.
    // DslParser zero-overhead: DSL line preserved, no metadata written.
    assert_eq!(result.content_blocks.len(), 2);
    assert!(
        matches!(
            &result.content_blocks[0],
            ContentBlock::Text(s)
                if s == "::button[label:OK;action:submit]"
        ),
        "DSL line must be preserved in zero-overhead passthrough mode"
    );
    assert!(matches!(&result.content_blocks[1], ContentBlock::Text(s) if s == "Plain response"));
    // DslParser zero-overhead: no dsl_result in metadata.
    assert!(
        !result.metadata.contains_key("dsl_result"),
        "incremental phase must not write dsl_result (zero-overhead passthrough)"
    );
}

/// Incremental phase with Off verbosity and no DSL: VerbosityFilter
/// removes non-Text blocks, DslParser passthrough finds no DSL,
/// so metadata must NOT contain dsl_result.
#[tokio::test]
async fn test_incremental_off_verbosity_no_dsl_no_metadata() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));

    let blocks = vec![
        thinking_block("hidden"),
        text_block("Just text"),
        tool_use_block("search"),
        tool_result_block("result"),
    ];
    let msg = closeclaw_common::processor::ProcessedMessage {
        content_blocks: blocks,
        metadata: make_meta("off"),
    };
    let result = registry.process_outbound_incremental(msg).await.unwrap();

    // Off: VerbosityFilter removes Thinking + ToolUse + ToolResult, keeps 1 Text.
    // DslParser passthrough: no DSL found → no dsl_result in metadata.
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Just text"));
    assert!(
        !result.metadata.contains_key("dsl_result"),
        "metadata must not contain dsl_result when no DSL is present"
    );
}
