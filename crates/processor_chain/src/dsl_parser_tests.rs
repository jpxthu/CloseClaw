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

    // DSL-only Text block is dropped after cleaning (DSL stripped → empty)
    assert_eq!(updated_blocks.len(), 5);
    assert!(matches!(&updated_blocks[0], ContentBlock::Text(s) if s == "Hello world"));
    assert!(matches!(&updated_blocks[1], ContentBlock::Thinking { .. }));
    assert!(matches!(&updated_blocks[2], ContentBlock::ToolUse { .. }));
    assert!(matches!(&updated_blocks[3], ContentBlock::Text(s) if s == "Done"));
    assert!(matches!(
        &updated_blocks[4],
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
    // DSL-only Text blocks are dropped after cleaning (empty after stripping DSL)
    assert_eq!(updated_blocks.len(), 0);
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
    // DSL lines are removed from Text blocks
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello"));
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

    // DSL lines are cleaned from content
    assert_eq!(result.text_content(), Some("Some text\nMore text"));
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

// ---------------------------------------------------------------------------
// Step 1.3: DSL line removal from Text blocks
// ---------------------------------------------------------------------------

#[test]
fn test_dsl_lines_removed_from_text_block() {
    let parser = DslParser;
    let blocks = vec![ContentBlock::Text(
        "Hello\n::button[label:X;action:a;value:1]\nWorld".to_string(),
    )];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    assert_eq!(result.instructions.len(), 1);
    assert_eq!(updated_blocks.len(), 1);
    assert!(matches!(
        &updated_blocks[0],
        ContentBlock::Text(s) if s == "Hello\nWorld"
    ));
}

#[test]
fn test_non_text_blocks_pass_through() {
    let parser = DslParser;
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "think".to_string(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "c1".to_string(),
            name: "fn".to_string(),
            input: "{}".to_string(),
        },
    ];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    assert!(result.instructions.is_empty());
    assert_eq!(updated_blocks.len(), 2);
    assert!(matches!(&updated_blocks[0], ContentBlock::Thinking { .. }));
    assert!(matches!(&updated_blocks[1], ContentBlock::ToolUse { .. }));
}

#[test]
fn test_empty_text_block_after_dsl_strip_is_dropped() {
    let parser = DslParser;
    let blocks = vec![ContentBlock::Text(
        "::button[label:A;action:x;value:1]".to_string(),
    )];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    assert_eq!(result.instructions.len(), 1);
    // DSL-only block → cleaned text is empty → dropped
    assert!(updated_blocks.is_empty());
}

#[test]
fn test_empty_string_text_block_is_dropped() {
    let parser = DslParser;
    let blocks = vec![ContentBlock::Text(String::new())];
    let (_, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    // Empty string → clean_lines is empty → clean_text is empty → dropped
    assert!(updated_blocks.is_empty());
}

#[test]
fn test_multiple_dsl_lines_in_single_text_block() {
    let parser = DslParser;
    let blocks = vec![ContentBlock::Text(
        "::button[label:A;action:x;value:1]\nSome text\n::button[label:B;action:y;value:2]"
            .to_string(),
    )];
    let (result, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    assert_eq!(result.instructions.len(), 2);
    assert_eq!(updated_blocks.len(), 1);
    assert!(matches!(&updated_blocks[0], ContentBlock::Text(s) if s == "Some text"));
}

#[test]
fn test_text_block_without_dsl_is_kept_intact() {
    let parser = DslParser;
    let blocks = vec![ContentBlock::Text("No DSL here".to_string())];
    let (_, updated_blocks) = parser.parse_content_blocks_with_result(&blocks);

    assert_eq!(updated_blocks.len(), 1);
    assert!(matches!(&updated_blocks[0], ContentBlock::Text(s) if s == "No DSL here"));
}

#[test]
fn test_parse_clean_text_removes_dsl() {
    let parser = DslParser;
    let input = "Hello\n::button[label:X;action:a;value:1]\nWorld";
    let (result, clean) = parser.parse(input);

    assert_eq!(result.instructions.len(), 1);
    assert_eq!(clean, "Hello\nWorld");
}

#[test]
fn test_parse_clean_text_all_dsl() {
    let parser = DslParser;
    let input = "::button[label:X;action:a;value:1]\n::selector[label:Y;action:b;options:A,B]";
    let (result, clean) = parser.parse(input);

    assert_eq!(result.instructions.len(), 2);
    assert_eq!(clean, "");
}

// ---------------------------------------------------------------------------
// Boundary / edge-case tests (Step 1.3)
// ---------------------------------------------------------------------------

#[test]
fn test_with_result_whitespace_only_text_preserved() {
    let parser = DslParser;
    let blocks = vec![ContentBlock::Text("  \n  ".to_string())];
    let (_, updated) = parser.parse_content_blocks_with_result(&blocks);
    // Whitespace-only text is non-empty → preserved.
    assert_eq!(updated.len(), 1);
}

#[test]
fn test_with_result_mixed_dsl_and_whitespace_only() {
    let parser = DslParser;
    let blocks = vec![
        ContentBlock::Text("Hello".to_string()),
        ContentBlock::Text("::button[label:A;action:x;value:1]".to_string()),
        ContentBlock::Text("  ".to_string()), // whitespace only
        ContentBlock::Text("World".to_string()),
    ];
    let (result, updated) = parser.parse_content_blocks_with_result(&blocks);
    assert_eq!(result.instructions.len(), 1);
    // DSL block dropped (empty after strip), whitespace block preserved.
    assert_eq!(updated.len(), 3);
    assert!(matches!(&updated[0], ContentBlock::Text(s) if s == "Hello"));
    assert!(matches!(&updated[1], ContentBlock::Text(s) if s == "  "));
    assert!(matches!(&updated[2], ContentBlock::Text(s) if s == "World"));
}

#[tokio::test]
async fn test_process_whitespace_only_content_blocks() {
    let parser = DslParser;
    let ctx = make_ctx("fallback", vec![ContentBlock::Text("  \n  ".to_string())]);
    let result = parser.process(&ctx).await.unwrap().unwrap();
    // Whitespace-only content_blocks → no DSL → original kept.
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "  \n  "));
}

#[tokio::test]
async fn test_process_empty_string_content_blocks() {
    let parser = DslParser;
    let ctx = make_ctx("fallback", vec![ContentBlock::Text("".to_string())]);
    let result = parser.process(&ctx).await.unwrap().unwrap();
    // Empty string content_blocks → empty after strip → falls back to ctx.content.
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "fallback"));
}

#[test]
fn test_with_result_instruction_ordering_preserved() {
    let parser = DslParser;
    let blocks = vec![
        ContentBlock::Text("::button[label:A;action:1;value:x]\n::selector[label:B;action:2]\n::button[label:C;action:3;value:y]".to_string()),
    ];
    let (result, _) = parser.parse_content_blocks_with_result(&blocks);
    assert_eq!(result.instructions.len(), 3);
    assert_eq!(result.instructions[0].instruction_type, "button");
    assert_eq!(result.instructions[0].params["label"], "A");
    assert_eq!(result.instructions[1].instruction_type, "selector");
    assert_eq!(result.instructions[1].params["label"], "B");
    assert_eq!(result.instructions[2].instruction_type, "button");
    assert_eq!(result.instructions[2].params["label"], "C");
}
