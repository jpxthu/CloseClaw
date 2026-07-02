//! Unit tests for DslParser — Step 1.6 (content_blocks support).

use crate::processor_chain::context::MessageContext;
use crate::processor_chain::dsl_parser::DslParser;
use crate::processor_chain::processor::MessageProcessor;
use closeclaw_llm::types::ContentBlock;

fn make_ctx(content: &str, content_blocks: Vec<ContentBlock>) -> MessageContext {
    MessageContext {
        content: content.to_string(),
        content_blocks,
        metadata: Default::default(),
        raw_message_log: vec![],
        skip: false,
    }
}

// ---------------------------------------------------------------------------
// parse_content_blocks_with_result tests
// ---------------------------------------------------------------------------

#[test]
fn test_with_result_mixed_blocks_preserves_non_text() {
    let parser = DslParser;
    let blocks = vec![
        ContentBlock::Text("Hello world".to_string()),
        ContentBlock::Thinking {
            thinking: "thinking...".to_string(),
            signature: None,
        },
        ContentBlock::Text("::button[label:A;action:x;value:1]".to_string()),
        ContentBlock::ToolUse {
            id: "call_1".to_string(),
            name: "search".to_string(),
            input: "{}".to_string(),
        },
        ContentBlock::Text("Done".to_string()),
        ContentBlock::ToolResult {
            tool_call_id: "call_1".to_string(),
            content: "result".to_string(),
        },
    ];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    // DSL instruction extracted from Text blocks
    assert_eq!(result.instructions.len(), 1);

    // All blocks preserved (text blocks kept as-is)
    assert_eq!(updated_blocks.len(), 6);
    assert!(matches!(&updated_blocks[0], ContentBlock::Text(s) if s == "Hello world"));
    assert!(matches!(&updated_blocks[1], ContentBlock::Thinking { .. }));
    assert!(matches!(
        &updated_blocks[2],
        ContentBlock::Text(s) if s == "::button[label:A;action:x;value:1]"
    ));
    assert!(matches!(&updated_blocks[3], ContentBlock::ToolUse { .. }));
    assert!(matches!(&updated_blocks[4], ContentBlock::Text(s) if s == "Done"));
    assert!(matches!(
        &updated_blocks[5],
        ContentBlock::ToolResult { .. }
    ));
}

#[test]
fn test_with_result_preserves_block_boundaries() {
    let parser = DslParser;
    let blocks = vec![
        ContentBlock::Text("Part1".to_string()),
        ContentBlock::Text("Part2".to_string()),
        ContentBlock::Text("Part3".to_string()),
    ];
    let (_, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    assert_eq!(updated_blocks.len(), 3);
    assert!(matches!(&updated_blocks[0], ContentBlock::Text(s) if s == "Part1"));
    assert!(matches!(&updated_blocks[1], ContentBlock::Text(s) if s == "Part2"));
    assert!(matches!(&updated_blocks[2], ContentBlock::Text(s) if s == "Part3"));
}

#[test]
fn test_with_result_dsl_text_blocks_preserved() {
    let parser = DslParser;
    let blocks = vec![
        ContentBlock::Text("::button[label:A;action:x;value:1]".to_string()),
        ContentBlock::Text("".to_string()),
        ContentBlock::Text("::button[label:B;action:y;value:2]".to_string()),
    ];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    assert_eq!(result.instructions.len(), 2);
    // Text blocks preserved as-is
    assert_eq!(updated_blocks.len(), 3);
}

#[test]
fn test_with_result_empty_input() {
    let parser = DslParser;
    let blocks: Vec<ContentBlock> = vec![];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);
    assert!(result.instructions.is_empty());
    assert!(updated_blocks.is_empty());
}

#[test]
fn test_with_result_only_non_text_blocks() {
    let parser = DslParser;
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "hmm".to_string(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "c1".to_string(),
            name: "fn".to_string(),
            input: "{}".to_string(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "c1".to_string(),
            content: "ok".to_string(),
        },
    ];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);
    assert!(result.instructions.is_empty());
    assert_eq!(updated_blocks.len(), 3);
}

// ---------------------------------------------------------------------------
// DslParser::process() branch tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_process_with_content_blocks_non_empty() {
    let parser = DslParser;
    let ctx = make_ctx(
        "fallback content",
        vec![
            ContentBlock::Text("Hello\n::button[label:X;action:a;value:1]".to_string()),
            ContentBlock::Thinking {
                thinking: "think".to_string(),
                signature: None,
            },
            ContentBlock::Text("World".to_string()),
        ],
    );
    let result = parser.process(&ctx).await.unwrap().unwrap();

    // When content_blocks is provided, DslParser processes them
    assert_eq!(result.content_blocks.len(), 3);
    assert!(
        matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello\n::button[label:X;action:a;value:1]")
    );
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(&result.content_blocks[2], ContentBlock::Text(s) if s == "World"));
}

#[tokio::test]
async fn test_process_with_empty_content_blocks_falls_back_to_content() {
    let parser = DslParser;
    let ctx = make_ctx(
        "Some text\n::button[label:A;action:x;value:1]\nMore text",
        vec![],
    );
    let result = parser.process(&ctx).await.unwrap().unwrap();

    assert_eq!(
        result.text_content(),
        Some("Some text\n::button[label:A;action:x;value:1]\nMore text")
    );
    // DSL instructions found → content block created
    assert_eq!(result.content_blocks.len(), 1);
}

#[tokio::test]
async fn test_process_pure_text_no_dsl_matches_pre_refactor() {
    let parser = DslParser;
    let ctx = make_ctx("Just a normal message", vec![]);
    let result = parser.process(&ctx).await.unwrap().unwrap();

    // No content_blocks → DslParser parses ctx.content, no DSL found
    // Returns content_blocks with the text as a Text block
    assert_eq!(result.text_content(), Some("Just a normal message"));
    // No DSL → still gets a Text block from the fallback
    assert_eq!(result.content_blocks.len(), 1);
}

#[tokio::test]
async fn test_process_content_blocks_takes_priority() {
    let parser = DslParser;
    let ctx = make_ctx(
        "::button[label:IGNORE;action:x;value:1]",
        vec![ContentBlock::Text("Actual content".to_string())],
    );
    let result = parser.process(&ctx).await.unwrap().unwrap();

    // content_blocks takes priority over ctx.content
    assert_eq!(result.text_content(), Some("Actual content"));
    let dsl_val = result.metadata.get("dsl_result").unwrap();
    assert!(!dsl_val.contains("button"));
}
