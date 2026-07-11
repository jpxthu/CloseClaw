//! Unit tests for [`VerbosityFilter`].

use super::processor::MessageProcessor;
use super::verbosity_filter::VerbosityFilter;
use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::ContentBlock;
use std::collections::HashMap;

fn thinking_block(thinking: &str) -> ContentBlock {
    ContentBlock::Thinking {
        thinking: thinking.to_string(),
        signature: None,
    }
}

fn text_block(text: &str) -> ContentBlock {
    ContentBlock::Text(text.to_string())
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

// -----------------------------------------------------------------------
// VerbosityFilter::filter tests
// -----------------------------------------------------------------------

#[test]
fn test_filter_full_passes_all() {
    let blocks = vec![
        text_block("hello"),
        thinking_block("thinking"),
        tool_use_block("search"),
        tool_result_block("result"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Full);
    assert_eq!(result.len(), 4);
}

#[test]
fn test_filter_normal_removes_thinking() {
    let blocks = vec![
        text_block("hello"),
        thinking_block("thinking"),
        text_block("world"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Normal);
    assert_eq!(result.len(), 2);
    assert!(matches!(result[0], ContentBlock::Text(_)));
    assert!(matches!(result[1], ContentBlock::Text(_)));
}

#[test]
fn test_filter_normal_keeps_tool_use_and_result() {
    let blocks = vec![
        text_block("hello"),
        thinking_block("thinking"),
        tool_use_block("search"),
        tool_result_block("result"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Normal);
    assert_eq!(result.len(), 3);
}

#[test]
fn test_filter_off_only_keeps_text() {
    let blocks = vec![
        text_block("hello"),
        thinking_block("thinking"),
        tool_use_block("search"),
        tool_result_block("result"),
        text_block("world"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 2);
    assert!(matches!(result[0], ContentBlock::Text(_)));
    assert!(matches!(result[1], ContentBlock::Text(_)));
}

#[test]
fn test_filter_empty_blocks() {
    let result = VerbosityFilter::filter(vec![], VerbosityLevel::Full);
    assert!(result.is_empty());
    let result = VerbosityFilter::filter(vec![], VerbosityLevel::Normal);
    assert!(result.is_empty());
    let result = VerbosityFilter::filter(vec![], VerbosityLevel::Off);
    assert!(result.is_empty());
}

// -----------------------------------------------------------------------
// VerbosityFilter metadata parsing
// -----------------------------------------------------------------------

#[test]
fn test_verbosity_from_metadata_with_valid_value() {
    let mut metadata = HashMap::new();
    metadata.insert("verbosity_level".to_string(), "normal".to_string());
    assert_eq!(
        VerbosityFilter::verbosity_from_metadata(&metadata),
        VerbosityLevel::Normal
    );
}

#[test]
fn test_verbosity_from_metadata_missing_defaults_to_normal() {
    let metadata = HashMap::new();
    assert_eq!(
        VerbosityFilter::verbosity_from_metadata(&metadata),
        VerbosityLevel::Normal
    );
}

#[test]
fn test_verbosity_from_metadata_invalid_defaults_to_normal() {
    let mut metadata = HashMap::new();
    metadata.insert("verbosity_level".to_string(), "invalid".to_string());
    assert_eq!(
        VerbosityFilter::verbosity_from_metadata(&metadata),
        VerbosityLevel::Normal
    );
}

// -----------------------------------------------------------------------
// VerbosityFilter trait impl tests (name, phase, priority)
// -----------------------------------------------------------------------

#[test]
fn test_name() {
    let f = VerbosityFilter;
    assert_eq!(f.name(), "verbosity_filter");
}

#[test]
fn test_phase() {
    let f = VerbosityFilter;
    assert_eq!(f.phase(), super::processor::ProcessPhase::Outbound);
}

#[test]
fn test_priority() {
    let f = VerbosityFilter;
    assert_eq!(f.priority(), 5);
}

// ── edge case tests ─────────────────────────────────────────────────

/// Normal mode with all Thinking blocks should produce an empty result.
#[test]
fn test_filter_normal_all_thinking_produces_empty() {
    let blocks = vec![
        thinking_block("think 1"),
        thinking_block("think 2"),
        thinking_block("think 3"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Normal);
    assert!(
        result.is_empty(),
        "Normal mode should remove all Thinking blocks"
    );
}

/// Off mode with mixed content types should keep only Text blocks.
#[test]
fn test_filter_off_mixed_content_keeps_only_text() {
    let blocks = vec![
        thinking_block("thinking"),
        text_block("hello"),
        tool_use_block("search"),
        tool_result_block("result"),
        text_block("world"),
        thinking_block("more thinking"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 2, "Off mode should keep only Text blocks");
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "hello"));
    assert!(matches!(&result[1], ContentBlock::Text(t) if t == "world"));
}

/// Normal mode preserves ToolUse and ToolResult alongside Text.
#[test]
fn test_filter_normal_preserves_tool_blocks() {
    let blocks = vec![
        text_block("before"),
        thinking_block("hidden"),
        tool_use_block("tool_a"),
        tool_result_block("result_a"),
        text_block("after"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Normal);
    assert_eq!(result.len(), 4, "should keep Text + ToolUse + ToolResult");
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "before"));
    assert!(matches!(&result[1], ContentBlock::ToolUse { .. }));
    assert!(matches!(&result[2], ContentBlock::ToolResult { .. }));
    assert!(matches!(&result[3], ContentBlock::Text(t) if t == "after"));
}

/// Full mode passes all block types without filtering.
#[test]
fn test_filter_full_no_filtering() {
    let blocks = vec![
        thinking_block("think"),
        text_block("text"),
        tool_use_block("tool"),
        tool_result_block("result"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Full);
    assert_eq!(result.len(), 4, "Full mode should not filter anything");
}
