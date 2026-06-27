//! Unit tests for verbosity filtering in outbound message routing.

use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::ContentBlock;

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
