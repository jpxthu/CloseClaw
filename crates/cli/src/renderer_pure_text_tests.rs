//! Tests for renderer pure text mode & design doc alignment (Step 1.6).
//!
//! Covers all 7 behavior dimensions specified in the plan:
//! 1. Pure text blockquote prefix preserved
//! 2. Pure text horizontal rule preserved
//! 3. Pure text heading marker preserved
//! 4. DSL hint lines no ANSI styles
//! 5. Code block backtick boundaries both modes
//! 6. ANSI mode regression verification
//! 7. Pure text inline markdown regression

use crate::renderer::{format_line, TerminalRenderer, BOLD, DIM, ITALIC};

use closeclaw_common::processor::{DslInstruction, DslParseResult};
use closeclaw_llm::types::ContentBlock;
use std::collections::HashMap;

// ── 1. Pure text blockquote prefix preserved ─────────────────────────────

/// Pure text mode: blockquote prefix `> ` preserved as-is.
#[test]
fn test_pure_text_blockquote_prefix_preserved() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("> hello".into()));
    assert_eq!(result.trim(), "> hello");
    assert!(!result.contains("│"));
}

/// Pure text mode: `format_line("> quote", false)` preserves prefix.
#[test]
fn test_pure_text_format_line_blockquote() {
    assert_eq!(format_line("> hello", false), "> hello");
}

// ── 2. Pure text horizontal rule preserved ───────────────────────────────

/// Pure text mode: horizontal rule `---` preserved as-is.
#[test]
fn test_pure_text_hr_preserved() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("---".into()));
    assert_eq!(result.trim(), "---");
    assert!(!result.contains("───"));
}

/// Pure text mode: `format_line("---", false)` returns `---`.
#[test]
fn test_pure_text_format_line_hr() {
    assert_eq!(format_line("---", false), "---");
}

/// Pure text mode: `render_markdown("---")` outputs `---`.
#[test]
fn test_pure_text_render_markdown_hr() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_markdown("---");
    assert!(result.contains("---"));
    assert!(!result.contains("───"));
}

// ── 3. Pure text heading marker preserved ────────────────────────────────

/// Pure text mode: heading marker `# ` preserved as-is.
#[test]
fn test_pure_text_heading_marker_preserved() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("# Heading".into()));
    assert_eq!(result.trim(), "# Heading");
    assert!(!result.contains(BOLD));
}

/// Pure text mode: `format_line("# Heading", false)` preserves `# ` prefix.
#[test]
fn test_pure_text_format_line_heading() {
    assert_eq!(format_line("# Heading", false), "# Heading");
    assert_eq!(format_line("## Sub", false), "## Sub");
}

// ── 4. DSL hint lines no ANSI styles ────────────────────────────────────

/// DSL hint lines: no ANSI styles in ANSI mode.
#[test]
fn test_dsl_hint_no_ansi_styles_ansi_mode() {
    let renderer = TerminalRenderer::with_ansi(true);
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".to_string(),
            params: HashMap::from([
                ("label".to_string(), "Go".to_string()),
                ("action".to_string(), "nav".to_string()),
                ("value".to_string(), "v".to_string()),
            ]),
        }],
    };
    let output = renderer.render(&[], Some(&dsl));
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("[Button:"));
    assert!(!text.contains(DIM));
    assert!(!text.contains(BOLD));
    assert!(!text.contains(ITALIC));
}

/// DSL hint lines: no ANSI styles in pure text mode.
#[test]
fn test_dsl_hint_no_ansi_styles_plain_mode() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".to_string(),
            params: HashMap::from([
                ("label".to_string(), "Go".to_string()),
                ("action".to_string(), "nav".to_string()),
                ("value".to_string(), "v".to_string()),
            ]),
        }],
    };
    let output = renderer.render(&[], Some(&dsl));
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("[Button:"));
    assert!(!text.contains(DIM));
    assert!(!text.contains(BOLD));
}

/// DSL selector hint: no ANSI styles in ANSI mode.
#[test]
fn test_dsl_selector_hint_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(true);
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
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("[Selector:"));
    assert!(!text.contains(DIM));
    assert!(!text.contains(BOLD));
}

// ── 5. Code block backtick boundaries both modes ────────────────────────

/// Code block backtick boundary lines: both ANSI and pure text modes.
#[test]
fn test_code_block_backtick_boundaries_both_modes() {
    for ansi in [false, true] {
        let renderer = TerminalRenderer::with_ansi(ansi);
        let result = renderer.render_code_block("rust", "fn main() {}");
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(
            lines[0], "```",
            "backtick boundary must be first line (ansi={})",
            ansi
        );
        assert_eq!(
            lines[lines.len() - 1],
            "```",
            "backtick boundary must be last line (ansi={})",
            ansi
        );
    }
}

/// Code block: backtick boundaries present with empty code.
#[test]
fn test_code_block_backtick_boundaries_empty_code() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_code_block("", "");
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines[0], "```");
    assert_eq!(lines[lines.len() - 1], "```");
}

// ── 6. ANSI mode regression verification ────────────────────────────────

/// ANSI mode: original heading bold behavior unchanged (regression).
#[test]
fn test_ansi_heading_bold_regression() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("# Title".into()));
    assert!(result.contains(BOLD));
    assert!(result.contains("Title"));
    assert!(!result.contains("# Title"));
}

/// ANSI mode: original blockquote dim+pipe behavior unchanged (regression).
#[test]
fn test_ansi_blockquote_regression() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("> quote".into()));
    assert!(result.contains(DIM));
    assert!(result.contains("│ quote"));
    assert!(!result.contains("> quote"));
}

/// ANSI mode: original hr dim+em-dash behavior unchanged (regression).
#[test]
fn test_ansi_hr_regression() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("---".into()));
    assert!(result.contains(DIM));
    assert!(result.contains("───"));
}

/// ANSI mode: bold inline markdown unchanged (regression).
#[test]
fn test_ansi_bold_inline_regression() {
    let renderer = TerminalRenderer::with_ansi(true);
    let output = renderer.render(&[ContentBlock::Text("**bold**".into())], None);
    let text = output.payload.as_str().unwrap();
    assert!(text.contains(BOLD));
}

/// ANSI mode: italic inline markdown unchanged (regression).
#[test]
fn test_ansi_italic_inline_regression() {
    let renderer = TerminalRenderer::with_ansi(true);
    let output = renderer.render(&[ContentBlock::Text("*italic*".into())], None);
    let text = output.payload.as_str().unwrap();
    assert!(text.contains(ITALIC));
}

/// ANSI mode: inline code unchanged (regression).
#[test]
fn test_ansi_inline_code_regression() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("`code`".into()));
    assert!(result.contains(BOLD));
    assert!(result.contains("code"));
}

/// ANSI mode: link rendering unchanged (regression).
#[test]
fn test_ansi_link_inline_regression() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("[text](http://example.com)".into()));
    assert!(result.contains("text"));
    assert!(result.contains("http://example.com"));
}

// ── 7. Pure text inline markdown regression ─────────────────────────────

/// Pure text mode: bold inline markdown — no ANSI codes (regression).
#[test]
fn test_pure_text_bold_inline_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("**bold**".into()));
    assert!(!result.contains(BOLD));
}

/// Pure text mode: italic inline markdown — no ANSI codes (regression).
#[test]
fn test_pure_text_italic_inline_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("*italic*".into()));
    assert!(!result.contains(ITALIC));
}

/// Pure text mode: inline code — no ANSI codes (regression).
#[test]
fn test_pure_text_inline_code_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("`code`".into()));
    assert!(!result.contains(BOLD));
    assert!(!result.contains(DIM));
    assert!(!result.contains(ITALIC));
}

/// Pure text mode: link rendering — no ANSI codes (regression).
#[test]
fn test_pure_text_link_inline_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("[text](http://example.com)".into()));
    assert!(result.contains("text"));
    assert!(result.contains("http://example.com"));
    assert!(!result.contains(DIM));
}
