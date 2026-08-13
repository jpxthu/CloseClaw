//! Step 1.7 — Tests for streaming incremental phase zero-overhead passthrough.
//!
//! Verifies that the outbound processor chain applied during the
//! streaming **finish** phase (post-stream pipeline) processes blocks
//! correctly: VerbosityFilter → DslParser → OutboundRawLog in order.
//!
//! The streaming **incremental** phase does NOT go through the processor
//! chain (blocks are sent directly by the Gateway), so these tests
//! validate the finish-phase chain behavior which is the only
//! processor-chain touchpoint in streaming.

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
    assert!(matches!(&result.content_blocks[0], ContentBlock::Thinking { .. }));
    assert!(matches!(&result.content_blocks[1], ContentBlock::Text(s) if s == "Hello"));
    assert!(matches!(&result.content_blocks[2], ContentBlock::Image { .. }));
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
    assert!(matches!(&result.content_blocks[1], ContentBlock::Image { .. }));
    assert!(matches!(&result.content_blocks[2], ContentBlock::Audio { .. }));
    assert!(matches!(&result.content_blocks[3], ContentBlock::File { .. }));
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
    assert!(matches!(&result.content_blocks[1], ContentBlock::ToolUse { .. }));
    assert!(matches!(&result.content_blocks[2], ContentBlock::ToolResult { .. }));
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
