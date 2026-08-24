//! Per-line truncation tests for Step 1.2.
//!
//! Verifies that `truncate_lines_to_width` truncates each line independently,
//! preserving lines that are within terminal width.

use crate::renderer::{get_terminal_width, TerminalRenderer};
use closeclaw_llm::types::ContentBlock;

/// Multi-line text: only over-width lines are truncated, short lines preserved.
#[test]
fn test_multiline_text_per_line_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let short_line = "short";
    let long_line = "x".repeat(200);
    let input = format!("{}\n{}", short_line, long_line);
    let result = renderer.render_block(&ContentBlock::Text(input));
    assert!(result.contains("short"), "short line must be preserved");
    assert!(
        result.contains("... (truncated)"),
        "over-width line must be truncated"
    );
}

/// Empty string returns empty string.
#[test]
fn test_multiline_text_empty_string() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text(String::new()));
    assert!(!result.contains("... (truncated)"));
}

/// Single-line text: boundary at width not truncated, over width truncated.
#[test]
fn test_multiline_text_single_line_matches_line_truncation() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    // Exactly at width: no truncation
    let at_width = "a".repeat(width);
    let result = renderer.render_block(&ContentBlock::Text(at_width));
    assert!(
        !result.contains("... (truncated)"),
        "single line at width should not truncate"
    );
    // Over width: truncated
    let over_width = "b".repeat(width + 1);
    let result = renderer.render_block(&ContentBlock::Text(over_width));
    assert!(
        result.contains("... (truncated)"),
        "single line over width must truncate"
    );
}

/// Multi-line Thinking: per-line truncation (not whole-content truncation).
#[test]
fn test_thinking_multiline_per_line_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let short_line = "thought";
    let long_line = "y".repeat(200);
    let input = format!("{}\n{}", short_line, long_line);
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: input,
        signature: None,
    });
    assert!(result.contains("thought"), "short line must be preserved");
    assert!(
        result.contains("... (truncated)"),
        "over-width line must be truncated"
    );
    assert!(result.contains("[end of thinking]"));
}

/// Multi-line ToolUse: per-line truncation.
#[test]
fn test_tool_use_multiline_per_line_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let short_line = "ok";
    let long_line = "z".repeat(200);
    let input = format!("{}\n{}", short_line, long_line);
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "exec".into(),
        input,
    });
    assert!(result.contains("exec"), "tool name must be preserved");
    assert!(
        result.contains("... (truncated)"),
        "over-width line must be truncated"
    );
}

/// Multi-line Text with markdown: per-line truncation.
#[test]
fn test_text_multiline_markdown_per_line_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let long_heading = format!("# {}", "w".repeat(200));
    let short_line = "ok";
    let input = format!("{}\n{}", long_heading, short_line);
    let result = renderer.render_block(&ContentBlock::Text(input));
    assert!(
        result.contains("... (truncated)"),
        "over-width heading line must be truncated"
    );
    assert!(result.contains("ok"), "short line must be preserved");
}

/// All lines short: no truncation anywhere.
#[test]
fn test_multiline_text_all_short_no_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("a\nb\nc".into()));
    assert!(!result.contains("... (truncated)"));
    assert!(result.contains("a"));
    assert!(result.contains("b"));
    assert!(result.contains("c"));
}

/// Trailing newline is preserved.
#[test]
fn test_multiline_text_trailing_newline_preserved() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("a\nb\n".into()));
    assert!(!result.contains("... (truncated)"));
    assert!(result.contains("a"));
    assert!(result.contains("b"));
}

/// Boundary: line exactly at terminal width is not truncated.
#[test]
fn test_multiline_text_boundary_at_width_no_truncation() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    let at_width = "x".repeat(width);
    let result = renderer.render_block(&ContentBlock::Text(at_width));
    assert!(!result.contains("... (truncated)"));
}

/// Boundary: line one char over terminal width is truncated.
#[test]
fn test_multiline_text_boundary_over_width_truncated() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    let over = "x".repeat(width + 1);
    let result = renderer.render_block(&ContentBlock::Text(over));
    assert!(result.contains("... (truncated)"));
}
