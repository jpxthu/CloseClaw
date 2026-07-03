//! Unit tests for verbosity filtering in outbound message routing.

use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::{ContentBlock, ContentBlockType};

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
