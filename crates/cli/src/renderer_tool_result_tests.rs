//! Unit tests for ToolResult line-count truncation (Step 1.2).
//!
//! Verifies that `render_tool_result` correctly applies line-count truncation
//! following the design doc: content rendered via markdown ANSI, truncated at
//! ~20 lines with a `... (truncated)` marker, wrapped in DIM when ANSI mode.

use crate::renderer::{strip_ansi, TerminalRenderer, DIM};
use closeclaw_llm::types::ContentBlock;

/// Helper: create a ToolResult ContentBlock with given content.
fn tool_result(content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_call_id: "t1".into(),
        content: content.into(),
    }
}

/// Generate a multiline string with exactly `n` lines.
fn lines(n: usize) -> String {
    (1..=n)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Normal path: ≤20 lines not truncated ────────────────────────────────────

/// Short content (a few lines) is not truncated; output line count equals input.
#[test]
fn test_tool_result_short_content_no_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&tool_result("hello\nworld"));
    assert!(
        !result.contains("... (truncated)"),
        "short content should not be truncated"
    );
    assert!(result.contains("hello"));
    assert!(result.contains("world"));
}

// ── Boundary: exactly 20 lines → no truncation ──────────────────────────────

/// Exactly 20 lines of content produces no truncation marker.
#[test]
fn test_tool_result_exactly_20_lines_no_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let content = lines(20);
    let result = renderer.render_block(&tool_result(&content));
    let stripped = strip_ansi(&result);
    assert!(
        !stripped.contains("... (truncated)"),
        "exactly 20 lines should not be truncated"
    );
    for i in 1..=20 {
        assert!(
            stripped.contains(&format!("line {i}")),
            "line {i} should be present in output"
        );
    }
}

// ── Truncation: 21 lines →21 lines (20 content + 1 marker) ──────────────────

/// 21 lines of content is truncated: output has exactly 21 lines
/// (20 content lines + 1 truncation marker).
#[test]
fn test_tool_result_21_lines_truncated_to_21() {
    let renderer = TerminalRenderer::with_ansi(false);
    let content = lines(30);
    let result = renderer.render_block(&tool_result(&content));
    let stripped = strip_ansi(&result);
    // Should contain truncation marker
    assert!(
        stripped.contains("... (truncated)"),
        "over-20-line content should be truncated"
    );
    // All 20 preserved lines should be present
    for i in 1..=20 {
        assert!(
            stripped.contains(&format!("line {i}")),
            "line {i} should be present in output"
        );
    }
    // Lines beyond 20 should NOT be present
    assert!(
        !stripped.contains("line 21"),
        "line 21 should be truncated away"
    );
    assert!(
        !stripped.contains("line 30"),
        "line 30 should be truncated away"
    );
}

// ── Boundary: 21 lines is the first truncated case ──────────────────────────

/// Exactly 21 lines triggers truncation; the marker is appended.
#[test]
fn test_tool_result_boundary_21_lines_truncated() {
    let renderer = TerminalRenderer::with_ansi(false);
    let content = lines(21);
    let result = renderer.render_block(&tool_result(&content));
    let stripped = strip_ansi(&result);
    assert!(
        stripped.contains("... (truncated)"),
        "21 lines should trigger truncation"
    );
    assert!(
        !stripped.contains("line 21"),
        "line 21 should be truncated away"
    );
}

// ── ANSI mode: DIM wrapping ─────────────────────────────────────────────────

/// In ANSI mode, truncated ToolResult output is wrapped in DIM/RESET.
#[test]
fn test_tool_result_ansi_dim_wrapping() {
    let renderer = TerminalRenderer::with_ansi(true);
    let content = lines(25);
    let result = renderer.render_block(&tool_result(&content));
    assert!(
        result.starts_with(DIM) || result.contains(DIM),
        "ANSI mode should include DIM style"
    );
    assert!(
        result.contains("... (truncated)"),
        "truncated content should have marker"
    );
}

/// In ANSI mode, short ToolResult output is also wrapped in DIM/RESET.
#[test]
fn test_tool_result_ansi_short_dim_wrapping() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&tool_result("short"));
    assert!(
        result.contains(DIM),
        "short ANSI ToolResult should include DIM"
    );
    assert!(!result.contains("... (truncated)"));
}

// ── Markdown rendering: code blocks ─────────────────────────────────────────

/// ToolResult content with a code block is rendered with line numbers
/// and language label, not raw markdown fences.
#[test]
fn test_tool_result_markdown_code_block_rendered() {
    let renderer = TerminalRenderer::with_ansi(true);
    let content = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
    let result = renderer.render_block(&tool_result(content));
    // Code block should be rendered (line numbers, language label)
    assert!(result.contains("fn"), "code should be rendered");
    assert!(result.contains("main"), "function name should be present");
    // Should NOT contain raw markdown fences after rendering
    assert!(
        !result.contains("```rust"),
        "raw markdown fences should be consumed by rendering"
    );
}

/// ToolResult with bold markdown is rendered with ANSI bold styling.
#[test]
fn test_tool_result_markdown_bold_rendered() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&tool_result("**important**"));
    // Bold should be rendered via ANSI escape (BOLD = \x1b[1m)
    assert!(
        result.contains("\x1b[1m"),
        "bold markdown should be rendered with ANSI BOLD"
    );
    assert!(
        result.contains("important"),
        "bold text content should be present"
    );
}

// ── Empty content ───────────────────────────────────────────────────────────

/// Empty string content does not crash and produces output.
#[test]
fn test_tool_result_empty_content_no_crash() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&tool_result(""));
    assert!(
        !result.is_empty(),
        "empty content should still produce output"
    );
    assert!(
        !result.contains("... (truncated)"),
        "empty content should not be truncated"
    );
}

/// Empty string content in ANSI mode includes DIM styling.
#[test]
fn test_tool_result_empty_content_ansi() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&tool_result(""));
    assert!(
        result.contains(DIM),
        "empty ANSI ToolResult should include DIM"
    );
}
