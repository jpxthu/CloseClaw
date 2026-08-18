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
fn test_filter_off_keeps_text_and_media() {
    let blocks = vec![
        text_block("hello"),
        thinking_block("thinking"),
        tool_use_block("search"),
        tool_result_block("result"),
        text_block("world"),
        ContentBlock::Image {
            name: "img.png".to_string(),
            url: "https://example.com/img.png".to_string(),
        },
        ContentBlock::Audio {
            name: "audio.wav".to_string(),
            url: "https://example.com/audio.wav".to_string(),
        },
        ContentBlock::File {
            name: "doc.pdf".to_string(),
            url: "https://example.com/doc.pdf".to_string(),
        },
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 5);
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "hello"));
    assert!(matches!(&result[1], ContentBlock::Text(t) if t == "world"));
    assert!(matches!(&result[2], ContentBlock::Image { .. }));
    assert!(matches!(&result[3], ContentBlock::Audio { .. }));
    assert!(matches!(&result[4], ContentBlock::File { .. }));
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

/// Off mode with mixed content types should keep only final reply blocks.
#[test]
fn test_filter_off_mixed_content_keeps_final_reply_and_media() {
    let blocks = vec![
        thinking_block("thinking"),
        text_block("hello"),
        tool_use_block("search"),
        tool_result_block("result"),
        text_block("world"),
        thinking_block("more thinking"),
        ContentBlock::Image {
            name: "img.png".to_string(),
            url: "https://example.com/img.png".to_string(),
        },
        ContentBlock::Audio {
            name: "audio.wav".to_string(),
            url: "https://example.com/audio.wav".to_string(),
        },
        ContentBlock::File {
            name: "doc.pdf".to_string(),
            url: "https://example.com/doc.pdf".to_string(),
        },
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert_eq!(
        result.len(),
        5,
        "Off mode should keep Text + Image + Audio + File"
    );
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "hello"));
    assert!(matches!(&result[1], ContentBlock::Text(t) if t == "world"));
    assert!(matches!(&result[2], ContentBlock::Image { .. }));
    assert!(matches!(&result[3], ContentBlock::Audio { .. }));
    assert!(matches!(&result[4], ContentBlock::File { .. }));
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

/// Off mode with only Image block should keep it (media blocks are always shown).
#[test]
fn test_filter_off_keeps_image_block() {
    let blocks = vec![ContentBlock::Image {
        name: "photo.jpg".to_string(),
        url: "https://example.com/photo.jpg".to_string(),
    }];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 1, "Off mode should keep Image blocks");
    assert!(matches!(&result[0], ContentBlock::Image { .. }));
}

/// Off mode with only Audio block should keep it (media blocks are always shown).
#[test]
fn test_filter_off_keeps_audio_block() {
    let blocks = vec![ContentBlock::Audio {
        name: "voice.mp3".to_string(),
        url: "https://example.com/voice.mp3".to_string(),
    }];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 1, "Off mode should keep Audio blocks");
    assert!(matches!(&result[0], ContentBlock::Audio { .. }));
}

/// Off mode with only File block should keep it (media blocks are always shown).
#[test]
fn test_filter_off_keeps_file_block() {
    let blocks = vec![ContentBlock::File {
        name: "report.csv".to_string(),
        url: "https://example.com/report.csv".to_string(),
    }];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 1, "Off mode should keep File blocks");
    assert!(matches!(&result[0], ContentBlock::File { .. }));
}

/// Off mode with all intermediate blocks should produce empty.
#[test]
fn test_filter_off_all_intermediate_produces_empty() {
    let blocks = vec![
        thinking_block("thinking"),
        tool_use_block("search"),
        tool_result_block("result"),
        thinking_block("more thinking"),
    ];
    let result = VerbosityFilter::filter(blocks, VerbosityLevel::Off);
    assert!(
        result.is_empty(),
        "Off mode should filter all intermediate blocks"
    );
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

// -----------------------------------------------------------------------
// Streaming consistency: should_keep_block / should_keep_thinking
// must agree with batch filter() for individual blocks
// -----------------------------------------------------------------------

/// Verify that `should_keep_block` returns the same result as `filter`
/// for every (block_type, verbosity_level) combination.
#[test]
fn test_should_keep_block_matches_filter_for_all_levels() {
    let all_blocks: Vec<ContentBlock> = vec![
        text_block("hello"),
        thinking_block("thinking"),
        tool_use_block("search"),
        tool_result_block("result"),
        ContentBlock::Image {
            name: "img.png".to_string(),
            url: "https://example.com/img.png".to_string(),
        },
        ContentBlock::Audio {
            name: "audio.wav".to_string(),
            url: "https://example.com/audio.wav".to_string(),
        },
        ContentBlock::File {
            name: "doc.pdf".to_string(),
            url: "https://example.com/doc.pdf".to_string(),
        },
    ];
    let levels = [
        VerbosityLevel::Full,
        VerbosityLevel::Normal,
        VerbosityLevel::Off,
    ];
    for level in levels {
        let batch_result = VerbosityFilter::filter(all_blocks.clone(), level);
        for block in &all_blocks {
            let per_block = VerbosityFilter::should_keep_block(block, level);
            let in_batch = batch_result
                .iter()
                .any(|b| std::mem::discriminant(b) == std::mem::discriminant(block));
            assert_eq!(
                per_block, in_batch,
                "should_keep_block mismatch for {:?} at {:?}",
                block, level
            );
        }
    }
}

/// Verify that `should_keep_thinking` is consistent with `filter` for
/// Thinking blocks across all verbosity levels.
#[test]
fn test_should_keep_thinking_matches_filter() {
    let levels = [
        VerbosityLevel::Full,
        VerbosityLevel::Normal,
        VerbosityLevel::Off,
    ];
    for level in levels {
        let batch = VerbosityFilter::filter(vec![thinking_block("t")], level);
        let per_block = VerbosityFilter::should_keep_thinking(level);
        assert_eq!(
            per_block,
            !batch.is_empty(),
            "should_keep_thinking({:?}) should match filter result",
            level
        );
    }
}

/// Verify batch filter and per-block filter produce the same block count
/// for a mixed content input at each verbosity level.
#[test]
fn test_streaming_batch_consistency_block_count() {
    let blocks = vec![
        thinking_block("think1"),
        text_block("text1"),
        thinking_block("think2"),
        text_block("text2"),
        tool_use_block("tool"),
    ];
    let levels = [
        VerbosityLevel::Full,
        VerbosityLevel::Normal,
        VerbosityLevel::Off,
    ];
    for level in levels {
        let batch = VerbosityFilter::filter(blocks.clone(), level);
        let per_block_count = blocks
            .iter()
            .filter(|b| VerbosityFilter::should_keep_block(b, level))
            .count();
        assert_eq!(
            batch.len(),
            per_block_count,
            "block count mismatch at {:?}",
            level
        );
    }
}
