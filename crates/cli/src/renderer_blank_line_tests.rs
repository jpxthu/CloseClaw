//! Blank line alignment tests for the TerminalRenderer component.
//!
//! Verifies the design-doc data flow requirements:
//! - Step 4: blank line between adjacent content blocks
//! - Step 5: blank line between DSL hint lines and body
//!
//! Separated from renderer_tests.rs to stay under the 1000-line limit.

use std::collections::HashMap;

use crate::renderer::{strip_ansi, TerminalRenderer};
use closeclaw_common::processor::{DslInstruction, DslParseResult};
use closeclaw_llm::types::ContentBlock;

// ── Helper ──────────────────────────────────────────────────────────────────

/// Helper: get raw payload string from RenderedOutput.
fn payload_text(output: &closeclaw_common::RenderedOutput) -> &str {
    output.payload.as_str().unwrap()
}

/// Count occurrences of `"\n\n"` in `text` — each represents a blank-line
/// separator (inter-block or DSL-body).
fn count_blank_lines(text: &str) -> usize {
    text.matches("\n\n").count()
}

// ── Normal path ─────────────────────────────────────────────────────────────

/// Multi-block (2+ blocks): blank line between adjacent blocks exists.
#[test]
fn test_blank_line_between_multiple_blocks() {
    let renderer = TerminalRenderer::with_ansi(false);
    let blocks = vec![
        ContentBlock::Text("first".into()),
        ContentBlock::Text("second".into()),
    ];
    let output = renderer.render(&blocks, None);
    let text = payload_text(&output);
    // "first\n" ends block 1, then "\n" is the blank line, then "second"
    assert!(
        text.contains("first\n\nsecond"),
        "expected blank line between blocks, got: {:?}",
        text
    );
}

/// DSL + single block: blank line between DSL hints and body.
#[test]
fn test_blank_line_between_dsl_and_single_block() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".to_string(),
            params: HashMap::from([
                ("label".to_string(), "OK".to_string()),
                ("action".to_string(), "confirm".to_string()),
                ("value".to_string(), "yes".to_string()),
            ]),
        }],
    };
    let blocks = vec![ContentBlock::Text("body".into())];
    let output = renderer.render(&blocks, Some(&dsl));
    let text = payload_text(&output);
    // DSL hint ends with "\n", then blank line "\n", then block starts
    assert!(
        text.contains("[Button:") && text.contains("\n\nbody"),
        "expected blank line between DSL and body, got: {:?}",
        text
    );
}

/// DSL + multi-block: both types of blank lines exist.
#[test]
fn test_blank_line_dsl_and_multi_block() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".to_string(),
            params: HashMap::from([
                ("label".to_string(), "Go".to_string()),
                ("action".to_string(), "nav".to_string()),
                ("value".to_string(), "url".to_string()),
            ]),
        }],
    };
    let blocks = vec![
        ContentBlock::Text("first".into()),
        ContentBlock::Text("second".into()),
    ];
    let output = renderer.render(&blocks, Some(&dsl));
    let text = payload_text(&output);
    // Blank line between DSL and first block
    assert!(
        text.contains("[Button:") && text.contains("\n\nfirst"),
        "expected blank line between DSL and first block, got: {:?}",
        text
    );
    // Blank line between first and second block
    assert!(
        text.contains("first\n\nsecond"),
        "expected blank line between first and second block, got: {:?}",
        text
    );
}

// ── Edge cases ──────────────────────────────────────────────────────────────

/// Single block, no DSL: no extra leading or trailing blank lines.
#[test]
fn test_single_block_no_dsl_no_extra_blank_lines() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[ContentBlock::Text("hello".into())], None);
    let text = payload_text(&output);
    // Should not start or end with blank line
    assert!(
        !text.starts_with('\n'),
        "should not have leading blank line, got: {:?}",
        text
    );
    assert!(
        !text.ends_with("\n\n"),
        "should not have trailing blank line, got: {:?}",
        text
    );
    assert!(text.contains("hello"));
}

/// Empty input (no blocks, no DSL): empty payload.
#[test]
fn test_empty_input_empty_payload() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[], None);
    let text = payload_text(&output);
    assert!(
        text.is_empty(),
        "empty input should produce empty payload, got: {:?}",
        text
    );
}

/// render_dsl returns empty string (empty instructions): no blank line insertion.
#[test]
fn test_empty_dsl_no_blank_line_insertion() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![],
    };
    let output = renderer.render(&[ContentBlock::Text("body".into())], Some(&dsl));
    let text = payload_text(&output);
    // No DSL output, so body should start directly
    assert!(
        text.starts_with("body"),
        "empty DSL should not cause blank line, got: {:?}",
        text
    );
}

/// Empty blocks + DSL only: output contains only DSL hint lines.
#[test]
fn test_empty_blocks_dsl_only() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "selector".to_string(),
            params: HashMap::from([
                ("label".to_string(), "Pick".to_string()),
                ("options".to_string(), "a,b".to_string()),
                ("action".to_string(), "select".to_string()),
            ]),
        }],
    };
    let output = renderer.render(&[], Some(&dsl));
    let text = payload_text(&output);
    assert!(text.contains("[Selector:"), "DSL selector must be present");
    assert!(
        !text.contains("\n\n"),
        "no blocks means no blank line insertion, got: {:?}",
        text
    );
}

// ── State transition: ANSI vs plain text ────────────────────────────────────

/// ANSI mode and plain text mode have consistent blank line structure.
#[test]
fn test_blank_line_ansi_vs_plain_text_consistency() {
    let blocks = vec![
        ContentBlock::Text("first".into()),
        ContentBlock::Text("second".into()),
    ];
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".to_string(),
            params: HashMap::from([
                ("label".to_string(), "OK".to_string()),
                ("action".to_string(), "go".to_string()),
                ("value".to_string(), "yes".to_string()),
            ]),
        }],
    };

    let ansi_renderer = TerminalRenderer::with_ansi(true);
    let plain_renderer = TerminalRenderer::with_ansi(false);

    let ansi_out = ansi_renderer.render(&blocks, Some(&dsl));
    let plain_out = plain_renderer.render(&blocks, Some(&dsl));

    // Strip ANSI from both
    let ansi_text = strip_ansi(payload_text(&ansi_out));
    let plain_text = payload_text(&plain_out);

    // Both should have blank line between DSL and body
    assert!(
        ansi_text.contains("[Button:") && ansi_text.contains("\n\nfirst"),
        "ANSI mode: blank line between DSL and first block, got: {:?}",
        ansi_text
    );
    assert!(
        plain_text.contains("[Button:") && plain_text.contains("\n\nfirst"),
        "plain mode: blank line between DSL and first block, got: {:?}",
        plain_text
    );
    // Both should have blank line between blocks
    assert!(
        ansi_text.contains("first\n\nsecond"),
        "ANSI mode: blank line between blocks, got: {:?}",
        ansi_text
    );
    assert!(
        plain_text.contains("first\n\nsecond"),
        "plain mode: blank line between blocks, got: {:?}",
        plain_text
    );
    // Stripped ANSI and plain should have identical blank line structure
    assert_eq!(
        ansi_text, plain_text,
        "blank line structure should be identical in both modes"
    );
}

// ── Long chain scenario ─────────────────────────────────────────────────────

/// 4-block sequence [Text, Thinking, ToolUse, ToolResult] → three blank lines.
#[test]
fn test_long_chain_four_blocks_three_blank_lines() {
    let renderer = TerminalRenderer::with_ansi(false);
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::Thinking {
            thinking: "thinking...".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "exec".into(),
            input: "ls".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "t1".into(),
            content: "output".into(),
        },
    ];
    let output = renderer.render(&blocks, None);
    let text = payload_text(&output);
    // 4 blocks → exactly 3 inter-block blank lines, no DSL-body blank line
    assert_eq!(
        count_blank_lines(text),
        3,
        "4 blocks should produce exactly 3 blank-line separators, got: {:?}",
        text
    );
    // Text block ends with "hello\n", blank line "\n", Thinking starts
    assert!(
        text.contains("hello\n\n"),
        "blank line between block 1 (Text) and block 2 (Thinking), got: {:?}",
        text
    );
    // Thinking ends with "[end of thinking]\n", blank line, ToolUse starts
    assert!(
        text.contains("[end of thinking]\n\n"),
        "blank line between block 2 (Thinking) and block 3 (ToolUse), got: {:?}",
        text
    );
    // ToolUse ends with ")\n", blank line, ToolResult starts
    assert!(
        text.contains(")\n\noutput"),
        "blank line between block 3 (ToolUse) and block 4 (ToolResult), got: {:?}",
        text
    );
    // Verify all blocks are present
    assert!(text.contains("hello"));
    assert!(text.contains("[Thinking]"));
    assert!(text.contains("exec"));
    assert!(text.contains("output"));
}

/// Long chain with DSL: DSL + 4 blocks → DSL-body blank line + 3 inter-block blank lines.
#[test]
fn test_long_chain_with_dsl() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".to_string(),
            params: HashMap::from([
                ("label".to_string(), "Run".to_string()),
                ("action".to_string(), "exec".to_string()),
                ("value".to_string(), "go".to_string()),
            ]),
        }],
    };
    let blocks = vec![
        ContentBlock::Text("intro".into()),
        ContentBlock::Thinking {
            thinking: "hmm".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "run".into(),
            input: "cmd".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "t1".into(),
            content: "done".into(),
        },
    ];
    let output = renderer.render(&blocks, Some(&dsl));
    let text = payload_text(&output);
    // DSL (1) + 4 blocks (3 inter-block) = 4 blank lines total
    assert_eq!(
        count_blank_lines(text),
        4,
        "DSL + 4 blocks should produce exactly 4 blank-line separators, got: {:?}",
        text
    );
    // DSL-body blank line
    assert!(
        text.contains("[Button:") && text.contains("\n\nintro"),
        "blank line between DSL and first block, got: {:?}",
        text
    );
    // Three inter-block blank lines
    assert!(text.contains("intro\n\n"), "blank line 1 (Text→Thinking)");
    assert!(
        text.contains("[end of thinking]\n\n"),
        "blank line 2 (Thinking→ToolUse)"
    );
    assert!(
        text.contains(")\n\ndone"),
        "blank line 3 (ToolUse→ToolResult)"
    );
}

// ── Multi-line DSL with multiple instructions ───────────────────────────────

/// Multiple DSL instructions + block: DSL-body blank line works with multi-line DSL.
#[test]
fn test_multi_line_dsl_blank_line_to_body() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![
            DslInstruction {
                instruction_type: "button".to_string(),
                params: HashMap::from([
                    ("label".to_string(), "A".to_string()),
                    ("action".to_string(), "a".to_string()),
                    ("value".to_string(), "1".to_string()),
                ]),
            },
            DslInstruction {
                instruction_type: "button".to_string(),
                params: HashMap::from([
                    ("label".to_string(), "B".to_string()),
                    ("action".to_string(), "b".to_string()),
                    ("value".to_string(), "2".to_string()),
                ]),
            },
        ],
    };
    let output = renderer.render(&[ContentBlock::Text("body".into())], Some(&dsl));
    let text = payload_text(&output);
    // 2 DSL instructions + 1 block → exactly 1 DSL-body blank line
    assert_eq!(
        count_blank_lines(text),
        1,
        "2-line DSL + 1 block should produce exactly 1 blank-line separator, got: {:?}",
        text
    );
    // DSL lines end with "\n", then blank line "\n", then body
    assert!(
        text.contains("[Button: B") && text.contains("\n\nbody"),
        "blank line after multi-line DSL and before body, got: {:?}",
        text
    );
    assert!(text.contains("[Button: A"));
    assert!(text.contains("[Button: B"));
}

// ── DSL-only (no blocks) no trailing blank line ─────────────────────────────

/// DSL with no content blocks: no trailing blank line after DSL.
#[test]
fn test_dsl_only_no_trailing_blank_line() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".to_string(),
            params: HashMap::from([
                ("label".to_string(), "OK".to_string()),
                ("action".to_string(), "go".to_string()),
                ("value".to_string(), "v".to_string()),
            ]),
        }],
    };
    let output = renderer.render(&[], Some(&dsl));
    let text = payload_text(&output);
    assert!(
        !text.ends_with("\n\n"),
        "DSL-only output should not have trailing blank line, got: {:?}",
        text
    );
    assert!(text.contains("[Button:"));
}

// ── Three blocks: exactly two blank lines ───────────────────────────────────

/// Three text blocks: exactly 2 blank line separators (N blocks → N-1 separators).
#[test]
fn test_three_blocks_two_blank_lines() {
    let renderer = TerminalRenderer::with_ansi(false);
    let blocks = vec![
        ContentBlock::Text("a".into()),
        ContentBlock::Text("b".into()),
        ContentBlock::Text("c".into()),
    ];
    let output = renderer.render(&blocks, None);
    let text = payload_text(&output);
    // 3 text blocks → exactly 2 inter-block blank lines
    assert_eq!(
        count_blank_lines(text),
        2,
        "3 blocks should produce exactly 2 blank-line separators, got: {:?}",
        text
    );
    assert!(text.contains("a\n\nb"), "blank line between a and b");
    assert!(text.contains("b\n\nc"), "blank line between b and c");
    // No blank line at start or end of body
    assert!(!text.starts_with('\n'));
}
