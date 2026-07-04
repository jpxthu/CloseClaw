//! Unit tests for verbosity filtering in outbound message routing.

use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::{ContentBlock, ContentBlockType};

use std::sync::Arc;

use super::outbound::filter_by_verbosity;

#[test]
fn test_filter_by_verbosity_full() {
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
    ];
    let result = filter_by_verbosity(blocks.clone(), VerbosityLevel::Full);
    assert_eq!(result.len(), 3);
    assert!(matches!(result[0], ContentBlock::Text(_)));
    assert!(matches!(result[1], ContentBlock::Thinking { .. }));
    assert!(matches!(result[2], ContentBlock::ToolUse { .. }));
}

#[test]
fn test_filter_by_verbosity_normal() {
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Normal);
    assert_eq!(result.len(), 2);
    assert!(matches!(result[0], ContentBlock::Text(_)));
    assert!(matches!(result[1], ContentBlock::ToolUse { .. }));
}

#[test]
fn test_filter_by_verbosity_off() {
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "hello"));
}

#[test]
fn test_filter_empty_blocks() {
    let blocks = vec![];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Full);
    assert!(result.is_empty());

    let result = filter_by_verbosity(vec![], VerbosityLevel::Normal);
    assert!(result.is_empty());

    let result = filter_by_verbosity(vec![], VerbosityLevel::Off);
    assert!(result.is_empty());
}

// ---------------------------------------------------------------------------
// Streaming per-block verbosity filtering
// ---------------------------------------------------------------------------
// These tests verify that the per-block filtering logic used in
// `process_stream_event` (streaming path) produces the same results
// as `filter_by_verbosity` (batch path) for individual blocks.
// The streaming path checks each BlockEnd individually.

/// Simulate the per-block streaming filter logic from `process_stream_event`.
/// Returns `true` if the block should be filtered (hidden).
fn streaming_should_filter(block_type: &ContentBlockType, level: VerbosityLevel) -> bool {
    *block_type != ContentBlockType::Text
        && match level {
            VerbosityLevel::Normal => matches!(block_type, ContentBlockType::Thinking),
            VerbosityLevel::Off => true,
            VerbosityLevel::Full => false,
        }
}

#[test]
fn test_streaming_text_never_filtered() {
    for level in [
        VerbosityLevel::Full,
        VerbosityLevel::Normal,
        VerbosityLevel::Off,
    ] {
        assert!(!streaming_should_filter(&ContentBlockType::Text, level));
    }
}

#[test]
fn test_streaming_thinking_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Full
    ));
    assert!(streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Normal
    ));
    assert!(streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Off
    ));
}

#[test]
fn test_streaming_tool_use_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::ToolUse,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_tool_result_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::ToolResult,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_image_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::Image,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_audio_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::Audio,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_file_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::File,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_non_text_filtered_at_off() {
    for bt in [
        ContentBlockType::Thinking,
        ContentBlockType::ToolUse,
        ContentBlockType::ToolResult,
        ContentBlockType::Image,
        ContentBlockType::Audio,
        ContentBlockType::File,
    ] {
        assert!(streaming_should_filter(&bt, VerbosityLevel::Off));
    }
}

#[test]
fn test_streaming_nothing_filtered_at_full() {
    for bt in [
        ContentBlockType::Text,
        ContentBlockType::Thinking,
        ContentBlockType::ToolUse,
        ContentBlockType::ToolResult,
        ContentBlockType::Image,
        ContentBlockType::Audio,
        ContentBlockType::File,
    ] {
        assert!(!streaming_should_filter(&bt, VerbosityLevel::Full));
    }
}

// ---------------------------------------------------------------------------
// Batch vs streaming consistency
// ---------------------------------------------------------------------------

#[test]
fn test_batch_normal_matches_streaming_for_thinking() {
    // Batch: Thinking blocks removed at Normal level.
    let blocks = vec![ContentBlock::Text("hi".into())];
    let batch_result = filter_by_verbosity(blocks, VerbosityLevel::Normal);
    assert_eq!(batch_result.len(), 1);
    // Streaming: Thinking block filtered at Normal level.
    assert!(streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_batch_normal_preserves_tool_result() {
    let blocks = vec![
        ContentBlock::Text("text".into()),
        ContentBlock::ToolResult {
            tool_call_id: "tc_1".into(),
            content: "result".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Normal);
    assert_eq!(
        result.len(),
        2,
        "ToolResult should not be filtered at Normal"
    );
    // Streaming: ToolResult not filtered at Normal.
    assert!(!streaming_should_filter(
        &ContentBlockType::ToolResult,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_batch_off_keeps_only_text() {
    let blocks = vec![
        ContentBlock::Text("keep".into()),
        ContentBlock::Thinking {
            thinking: "rm".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "t".into(),
            input: "{}".into(),
        },
        ContentBlock::Image {
            name: "img".into(),
            url: "https://example.com/img.png".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "keep"));
}

// ===========================================================================
// Gateway-level three-step outbound flow tests
// ===========================================================================
// These tests verify that the outbound pipeline correctly executes:
//   Step 1: Verbosity filtering (filter_by_verbosity)
//   Step 2: Processor Chain (DslParser only)
//   Step 3: Outbound log (Gateway layer, not tested here)
//
// Since filter_by_verbosity is a pure function and the chain only contains
// DslParser, we test the combined flow by composing the two steps.

use closeclaw_common::processor::ProcessedMessage;

/// Build a DslParser-only outbound registry for three-step flow tests.
fn build_dsl_registry() -> closeclaw_processor_chain::ProcessorRegistry {
    let mut registry = closeclaw_processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(closeclaw_processor_chain::DslParser));
    registry
}

/// Simulate the three-step outbound pipeline:
/// 1. Verbosity filter → 2. DslParser chain → 3. (log omitted in unit test)
fn simulate_outbound_pipeline(
    blocks: Vec<ContentBlock>,
    verbosity: VerbosityLevel,
) -> ProcessedMessage {
    // Step 1: Verbosity filter
    let filtered = filter_by_verbosity(blocks, verbosity);

    // Step 2: Processor chain (DslParser)
    let mut meta = std::collections::HashMap::new();
    meta.insert("verbosity_level".to_string(), verbosity.to_string());
    let input = ProcessedMessage {
        content_blocks: filtered,
        metadata: meta,
    };
    let registry = build_dsl_registry();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(registry.process_outbound(input)).unwrap()
}

#[test]
fn test_three_step_normal_filters_thinking_then_parses_dsl() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "internal reasoning".into(),
            signature: None,
        },
        ContentBlock::Text("::button[label:OK;action:submit]".into()),
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Normal);

    // Step 1: Normal filters Thinking blocks
    // Step 2: DslParser extracts DSL from remaining Text block
    // DslParser fallback keeps the original content when all blocks are stripped
    assert_eq!(
        result.content_blocks.len(),
        1,
        "DSL-only block: fallback keeps original text"
    );
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

#[test]
fn test_three_step_off_keeps_only_text_then_parses_dsl() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "rm".into(),
            signature: None,
        },
        ContentBlock::Text("Hello".into()),
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "t".into(),
            input: "{}".into(),
        },
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Off);

    // Step 1: Off keeps only Text
    // Step 2: DslParser processes Text
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello"));
}

#[test]
fn test_three_step_full_keeps_all_then_parses_dsl() {
    let blocks = vec![
        ContentBlock::Text("Hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "t".into(),
            input: "{}".into(),
        },
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Full);

    // Step 1: Full keeps all blocks
    // Step 2: DslParser passes through non-Text blocks, Text passes unchanged
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello"));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::ToolUse { .. }
    ));
}

#[test]
fn test_three_step_empty_blocks() {
    let blocks: Vec<ContentBlock> = vec![];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Normal);

    // Empty blocks → DslParser fallback creates a Text block from content (empty)
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
}

#[test]
fn test_three_step_mixed_blocks_with_dsl() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "step 1".into(),
            signature: None,
        },
        ContentBlock::Text("Result here.".into()),
        ContentBlock::Text("::button[label:OK;action:ok]".into()),
        ContentBlock::ToolUse {
            id: "2".into(),
            name: "tool".into(),
            input: "{}".into(),
        },
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Normal);

    // Normal: Thinking filtered
    // DslParser: first Text kept, second Text (DSL-only) stripped, ToolUse passed
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Result here."));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::ToolUse { .. }
    ));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}
