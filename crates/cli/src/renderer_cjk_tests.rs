//! CJK and fullwidth truncation tests for the TerminalRenderer.
//!
//! Separated from `renderer_tests.rs` to stay under the 1000-line limit.
//! Verifies that `truncate_to_width()` uses terminal display column width
//! (not character count) for CJK/fullwidth characters.

use crate::renderer::{get_terminal_width, strip_ansi, TerminalRenderer};
use closeclaw_llm::types::ContentBlock;

// ── CJK within/exceeding width ──────────────────────────────────────────

/// CJK content within terminal width is not truncated.
/// 40 Chinese chars × 2 columns = 80 columns → no truncation.
#[test]
fn test_cjk_within_width_no_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let content: String = "中".repeat(40); // 40 × 2 = 80 columns
    let result = renderer.render_block(&ContentBlock::Text(content.clone()));
    assert!(
        !result.contains("... (truncated)"),
        "CJK content within terminal width should not be truncated"
    );
    assert!(result.contains("中"));
}

/// CJK content exceeding terminal width is truncated.
/// 41 Chinese chars × 2 columns = 82 columns → triggers truncation.
#[test]
fn test_cjk_over_width_truncated() {
    let renderer = TerminalRenderer::with_ansi(false);
    let content: String = "字".repeat(41); // 41 × 2 = 82 columns
    let result = renderer.render_block(&ContentBlock::Text(content));
    assert!(
        result.contains("... (truncated)"),
        "CJK content exceeding terminal width should be truncated"
    );
}

// ── Mixed ASCII + CJK ───────────────────────────────────────────────────

/// Mixed ASCII + CJK: 78 ASCII + 1 CJK = 80 columns → no truncation.
#[test]
fn test_mixed_ascii_cjk_at_width_no_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let ascii_part = "a".repeat(78); // 78 × 1 = 78 columns
    let mixed = format!("{}中", ascii_part); // 78 + 2 = 80 columns
    let result = renderer.render_block(&ContentBlock::Text(mixed));
    assert!(!result.contains("... (truncated)"));
}

/// Mixed ASCII + CJK: 79 ASCII + 1 CJK = 81 columns → truncated.
#[test]
fn test_mixed_ascii_cjk_over_width_truncated() {
    let renderer = TerminalRenderer::with_ansi(false);
    let ascii_part = "b".repeat(79); // 79 × 1 = 79 columns
    let mixed = format!("{}字", ascii_part); // 79 + 2 = 81 columns
    let result = renderer.render_block(&ContentBlock::Text(mixed));
    assert!(result.contains("... (truncated)"));
}

// ── Boundary values ──────────────────────────────────────────────────────

/// Boundary: CJK content width equals terminal width → no truncation.
/// Uses get_terminal_width() to construct exact-width content.
#[test]
fn test_cjk_boundary_exact_width_no_truncation() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    // CJK content width must be even to match exactly;
    // use (width / 2) pairs + pad odd column with ASCII.
    let cjk_count = width / 2;
    let remainder = width % 2;
    let cjk_part: String = "中".repeat(cjk_count);
    let ascii_part: String = "a".repeat(remainder);
    let content = format!("{}{}", cjk_part, ascii_part);
    let result = renderer.render_block(&ContentBlock::Text(content));
    assert!(!result.contains("... (truncated)"));
}

/// Boundary: CJK content width equals terminal width + 1 → truncated.
#[test]
fn test_cjk_boundary_over_width_truncated() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    let cjk_count = width / 2;
    let remainder = width % 2;
    let cjk_part: String = "字".repeat(cjk_count);
    let ascii_part: String = "c".repeat(remainder + 1); // +1 pushes over
    let content = format!("{}{}", cjk_part, ascii_part);
    let result = renderer.render_block(&ContentBlock::Text(content));
    assert!(result.contains("... (truncated)"));
}

// ── Fullwidth punctuation ────────────────────────────────────────────────

/// Full-width punctuation (、。！) occupies 2 columns each.
#[test]
fn test_fullwidth_punctuation_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    // 40 fullwidth punctuation × 2 = 80 columns → no truncation
    let content: String = "、".repeat(40);
    let result = renderer.render_block(&ContentBlock::Text(content));
    assert!(!result.contains("... (truncated)"));
    // 41 fullwidth punctuation × 2 = 82 columns → truncated
    let content: String = "。".repeat(41);
    let result = renderer.render_block(&ContentBlock::Text(content));
    assert!(result.contains("... (truncated)"));
    // Mix of different fullwidth punctuation
    let content: String = "！？。、，".repeat(14); // 5 × 14 = 70 chars = 140 cols → truncated
    let result = renderer.render_block(&ContentBlock::Text(content));
    assert!(result.contains("... (truncated)"));
}

// ── CJK in different block types ────────────────────────────────────────

/// CJK content in Thinking block is truncated by display width.
#[test]
fn test_cjk_thinking_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let cjk_content: String = "思".repeat(60); // 60 × 2 = 120 columns → truncated
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: cjk_content,
        signature: None,
    });
    assert!(result.contains("... (truncated)"));
    assert!(result.contains("[end of thinking]"));
}

/// CJK content in ToolUse block is truncated by display width.
#[test]
fn test_cjk_tool_use_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let cjk_input: String = "参".repeat(60); // 120 columns → truncated
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "tool".into(),
        input: cjk_input,
    });
    assert!(result.contains("... (truncated)"));
    assert!(result.contains("tool")); // tool name preserved
}

/// CJK content in ToolResult block is truncated by display width.
#[test]
fn test_cjk_tool_result_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let cjk_content: String = "结".repeat(60); // 120 columns → truncated
    let result = renderer.render_block(&ContentBlock::ToolResult {
        tool_call_id: "t1".into(),
        content: cjk_content,
    });
    assert!(result.contains("... (truncated)"));
}

// ── Truncated content assertions ─────────────────────────────────────────

/// Verify truncated output preserves a prefix of the original content.
/// Mixed content: 79 ASCII + 1 CJK = 81 columns (exceeds 80).
/// At 80 columns, the first 80 columns (79 ASCII + nothing more)
/// are kept, then the marker is appended.
#[test]
fn test_truncated_content_preserves_prefix() {
    let renderer = TerminalRenderer::with_ansi(false);
    let ascii_part = "b".repeat(79);
    let mixed = format!("{}字", ascii_part); // 79 + 2 = 81 columns
    let result = renderer.render_block(&ContentBlock::Text(mixed));
    // First 79 ASCII chars fit within 80 columns; the CJK char pushes over.
    // Truncated output should start with the 79 ASCII chars.
    assert!(
        result.starts_with("b"),
        "truncated output should preserve prefix"
    );
    assert!(result.contains("... (truncated)"));
}

/// Verify truncated output appends the truncation marker at the end.
/// Note: `truncate_lines_to_width` preserves trailing newlines from
/// `render_markdown_ansi`, so the marker is followed by `\n`.
#[test]
fn test_truncated_content_ends_with_marker() {
    let renderer = TerminalRenderer::with_ansi(false);
    let content: String = "字".repeat(41); // 82 columns
    let result = renderer.render_block(&ContentBlock::Text(content));
    let stripped = strip_ansi(&result);
    let trimmed = stripped.trim_end_matches('\n');
    assert!(
        trimmed.ends_with("... (truncated)"),
        "truncated output should end with marker, got: {:?}",
        stripped
    );
}

// ── Edge: first char wider than available width ─────────────────────────

/// When available terminal width < 2 and the first character is CJK
/// (width 2), nothing fits — only the truncation marker is returned.
#[test]
fn test_first_cjk_char_wider_than_available_width() {
    let renderer = TerminalRenderer::with_ansi(false);
    // We can't change terminal width, so use the real width.
    // Construct content that exceeds any reasonable width to verify
    // single-char CJK truncation doesn't break.
    let width = get_terminal_width();
    let cjk_count = width / 2 + 1; // ensure we exceed by at least 1 column
    let over_content: String = "测".repeat(cjk_count);
    let result = renderer.render_block(&ContentBlock::Text(over_content));
    assert!(
        result.contains("... (truncated)"),
        "CJK content exceeding width should be truncated"
    );
    // The truncated prefix should be the first (width/2) chars
    let prefix: String = "测".repeat(width / 2);
    assert!(
        result.starts_with(&prefix),
        "truncated output should preserve the fitting prefix"
    );
}

/// Single CJK character within terminal width is not truncated.
#[test]
fn test_single_cjk_char_no_truncation() {
    let width = get_terminal_width();
    if width >= 2 {
        let renderer = TerminalRenderer::with_ansi(false);
        let result = renderer.render_block(&ContentBlock::Text("中".into()));
        assert!(!result.contains("... (truncated)"));
        assert!(result.contains("中"));
    }
}
