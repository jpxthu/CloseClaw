//! Unit tests for the TerminalRenderer component.
//!
//! Covers direct rendering of content blocks, DSL elements, markdown,
//! code blocks, ANSI helpers, and edge cases.

use crate::renderer::{
    check_line_pattern, get_terminal_width, resolve_terminal_width_from, strip_ansi,
    TerminalRenderer, BOLD, CYAN, DIM, ITALIC,
};
use closeclaw_common::processor::{DslInstruction, DslParseResult};
use closeclaw_llm::types::ContentBlock;

// ── Constructor tests ───────────────────────────────────────────────────────

#[test]
fn test_renderer_new() {
    let _renderer = TerminalRenderer::new();
}

#[test]
fn test_renderer_with_ansi_true() {
    let renderer = TerminalRenderer::with_ansi(true);
    // Smoke test: render simple text
    let output = renderer.render(&[ContentBlock::Text("hi".into())], None);
    assert_eq!(output.msg_type, "text");
}

#[test]
fn test_renderer_with_ansi_false() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[ContentBlock::Text("hi".into())], None);
    assert_eq!(output.msg_type, "text");
}

// ── render_block tests ──────────────────────────────────────────────────────

#[test]
fn test_render_block_text() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("hello".into()));
    assert!(result.contains("hello"));
}

#[test]
fn test_render_block_thinking_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: "reasoning".into(),
        signature: None,
    });
    assert!(result.contains("[Thinking]"));
    assert!(result.contains("reasoning"));
    assert!(result.contains("[end of thinking]"));
}

#[test]
fn test_render_block_thinking_ansi() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: "thought".into(),
        signature: None,
    });
    assert!(result.contains(DIM));
    assert!(result.contains("[Thinking]"));
    assert!(result.contains("[end of thinking]"));
}

#[test]
fn test_render_block_tool_use_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "exec".into(),
        input: "ls -la".into(),
    });
    assert!(result.contains("exec"));
    assert!(result.contains("ls -la"));
    assert!(result.contains("⚙"));
}

#[test]
fn test_render_block_tool_use_ansi() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "exec".into(),
        input: "pwd".into(),
    });
    assert!(result.contains(DIM));
    assert!(result.contains(BOLD));
    assert!(result.contains(CYAN));
}

#[test]
fn test_render_block_tool_result_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::ToolResult {
        tool_call_id: "t1".into(),
        content: "output data".into(),
    });
    assert!(result.contains("output data"));
}

#[test]
fn test_render_block_tool_result_ansi() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::ToolResult {
        tool_call_id: "t1".into(),
        content: "result".into(),
    });
    assert!(result.contains(DIM));
}

#[test]
fn test_render_block_image_placeholder() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Image("photo.png".into()));
    assert!(result.contains("[image: photo.png]"));
}

#[test]
fn test_render_block_audio_placeholder() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Audio("voice.wav".into()));
    assert!(result.contains("[audio: voice.wav]"));
}

#[test]
fn test_render_block_file_placeholder() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::File("doc.pdf".into()));
    assert!(result.contains("[file: doc.pdf]"));
}

// ── render() tests ──────────────────────────────────────────────────────────

#[test]
fn test_render_single_text_block() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[ContentBlock::Text("hello".into())], None);
    assert_eq!(output.msg_type, "text");
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("hello"));
}

#[test]
fn test_render_empty_blocks() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[], None);
    assert_eq!(output.msg_type, "text");
}

#[test]
fn test_render_multiple_blocks() {
    let renderer = TerminalRenderer::with_ansi(false);
    let blocks = vec![
        ContentBlock::Text("first".into()),
        ContentBlock::Text("second".into()),
    ];
    let output = renderer.render(&blocks, None);
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("first"));
    assert!(text.contains("second"));
}

#[test]
fn test_render_mixed_block_types() {
    let renderer = TerminalRenderer::with_ansi(false);
    let blocks = vec![
        ContentBlock::Text("intro".into()),
        ContentBlock::ToolUse {
            id: "c1".into(),
            name: "run".into(),
            input: "echo hi".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "c1".into(),
            content: "hi".into(),
        },
    ];
    let output = renderer.render(&blocks, None);
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("intro"));
    assert!(text.contains("run"));
    assert!(text.contains("hi"));
}

#[test]
fn test_render_with_dsl_button() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        clean_content: String::new(),
        instructions: vec![DslInstruction::Button {
            label: "Click".into(),
            action: "go".into(),
            value: "ok".into(),
        }],
    };
    let output = renderer.render(&[ContentBlock::Text("body".into())], Some(&dsl));
    let text = output.payload.as_str().unwrap();
    assert!(
        text.contains("[Button:"),
        "DSL button should appear in output"
    );
    assert!(text.contains("body"));
}

#[test]
fn test_render_with_dsl_selector() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        clean_content: String::new(),
        instructions: vec![DslInstruction::Selector {
            label: "Pick one".into(),
            options: vec!["a".into(), "b".into()],
            action: "select".into(),
        }],
    };
    let output = renderer.render(&[], Some(&dsl));
    let text = output.payload.as_str().unwrap();
    assert!(
        text.contains("[Selector:"),
        "DSL selector should appear in output"
    );
}

#[test]
fn test_render_ansi_strips_codes_when_disabled() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[ContentBlock::Text("**bold**".into())], None);
    let text = output.payload.as_str().unwrap();
    assert!(!text.contains(BOLD), "ansi should not contain BOLD escape");
}

#[test]
fn test_render_ansi_contains_codes_when_enabled() {
    let renderer = TerminalRenderer::with_ansi(true);
    let output = renderer.render(&[ContentBlock::Text("**bold**".into())], None);
    let text = output.payload.as_str().unwrap();
    assert!(text.contains(BOLD), "ansi should contain BOLD escape");
}

// ── render_code_block tests ─────────────────────────────────────────────────

#[test]
fn test_render_code_block_rust() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_code_block("rust", "fn main() {}");
    assert!(result.contains("fn"));
    assert!(result.contains("main"));
}

#[test]
fn test_render_code_block_unknown_language() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_code_block("cobol", "DISPLAY 'hello'");
    assert!(result.contains("```"));
}

#[test]
fn test_render_code_block_with_line_numbers() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_code_block("rust", "line1\nline2\nline3");
    assert!(result.contains("1"));
    assert!(result.contains("2"));
    assert!(result.contains("3"));
}

// ── render_markdown tests ───────────────────────────────────────────────────

#[test]
fn test_render_markdown_heading() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_markdown("# Title");
    assert!(result.contains("Title"));
}

#[test]
fn test_render_markdown_blockquote() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_markdown("> quote");
    assert!(result.contains("│ quote"));
}

#[test]
fn test_render_markdown_hr() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_markdown("---");
    assert!(result.contains("───"));
}

#[test]
fn test_render_markdown_bold() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_markdown("**bold**");
    assert!(result.contains(BOLD));
}

#[test]
fn test_render_markdown_italic() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_markdown("*italic*");
    assert!(result.contains(ITALIC));
}

// ── render_hr tests ─────────────────────────────────────────────────────────

#[test]
fn test_render_hr_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_hr();
    assert_eq!(result, "───");
}

#[test]
fn test_render_hr_ansi() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_hr();
    assert!(result.contains(DIM));
    assert!(result.contains("───"));
}

// ── strip_ansi tests ────────────────────────────────────────────────────────

#[test]
fn test_strip_ansi_removes_escapes() {
    let input = "\x1b[1mhello\x1b[0m";
    assert_eq!(strip_ansi(input), "hello");
}

#[test]
fn test_strip_ansi_no_escapes() {
    assert_eq!(strip_ansi("plain text"), "plain text");
}

#[test]
fn test_strip_ansi_empty() {
    assert_eq!(strip_ansi(""), "");
}

#[test]
fn test_strip_ansi_multiple_codes() {
    let input = "\x1b[36m\x1b[1mcyan bold\x1b[0m";
    assert_eq!(strip_ansi(input), "cyan bold");
}

// ── Edge cases ──────────────────────────────────────────────────────────────

#[test]
fn test_render_text_empty_string() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[ContentBlock::Text(String::new())], None);
    assert_eq!(output.msg_type, "text");
}

#[test]
fn test_render_thinking_empty() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: String::new(),
        signature: None,
    });
    assert!(result.contains("[Thinking]"));
}

#[test]
fn test_render_tool_use_empty_input() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "noop".into(),
        input: String::new(),
    });
    assert!(result.contains("noop"));
}

#[test]
fn test_render_tool_result_empty() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::ToolResult {
        tool_call_id: "t1".into(),
        content: String::new(),
    });
    // Empty result still renders (with newline)
    assert!(!result.is_empty());
}

#[test]
fn test_streaming_renderer_access() {
    let renderer = TerminalRenderer::new();
    let sr = renderer.streaming_renderer();
    // Just verify we can lock and get a reference
    let _lock = sr.lock().unwrap();
}

// ── get_terminal_width / resolve_terminal_width_from tests ────────────────

/// Normal path: terminal size is available → return actual width.
#[test]
fn test_resolve_terminal_width_from_some_returns_width() {
    assert_eq!(resolve_terminal_width_from(Some((120, 40))), 120);
    assert_eq!(resolve_terminal_width_from(Some((80, 24))), 80);
    assert_eq!(resolve_terminal_width_from(Some((200, 60))), 200);
}

/// Fallback path: no terminal → return 80 (documented default).
#[test]
fn test_resolve_terminal_width_from_none_returns_80() {
    assert_eq!(resolve_terminal_width_from(None), 80);
}

/// Edge case: terminal reports zero width → return 0 (not 80).
/// Zero width is technically a valid `Some` value from `terminal_size`.
#[test]
fn test_resolve_terminal_width_from_zero_returns_zero() {
    assert_eq!(resolve_terminal_width_from(Some((0, 0))), 0);
}

/// Integration: `get_terminal_width()` always returns a positive value.
/// In CI (no terminal) this tests the fallback; locally it may test the
/// actual terminal path.
#[test]
fn test_get_terminal_width_returns_positive() {
    let width = get_terminal_width();
    assert!(width > 0, "get_terminal_width() should always be > 0");
}

// ── Truncation tests (Step 1.2) ───────────────────────────────────────────

/// render_thinking truncation — short content passes through unchanged.
#[test]
fn test_render_thinking_short_no_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let short = "hello";
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: short.into(),
        signature: None,
    });
    assert!(result.contains("hello"));
    assert!(
        !result.contains("... (truncated)"),
        "short thinking should not be truncated"
    );
    assert!(result.contains("[end of thinking]"));
}

/// render_thinking truncation — content exactly at terminal width is not truncated.
#[test]
fn test_render_thinking_boundary_no_truncation() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    let content: String = "x".repeat(width);
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: content,
        signature: None,
    });
    assert!(
        !result.contains("... (truncated)"),
        "content at terminal width should not be truncated"
    );
    assert!(result.contains("[end of thinking]"));
}

/// render_thinking truncation — content exceeding terminal width is truncated.
#[test]
fn test_render_thinking_boundary_overflows_truncation() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    let content: String = "x".repeat(width + 1);
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: content,
        signature: None,
    });
    assert!(
        result.contains("... (truncated)"),
        "overwidth thinking should be truncated"
    );
    assert!(result.contains("[end of thinking]"));
}

/// render_thinking truncation — long text shows truncated marker and boundaries.
#[test]
fn test_render_thinking_long_truncated_with_end_marker() {
    let renderer = TerminalRenderer::with_ansi(false);
    let long_text = "a".repeat(200);
    let result = renderer.render_block(&ContentBlock::Thinking {
        thinking: long_text,
        signature: None,
    });
    assert!(result.contains("[Thinking]"));
    assert!(result.contains("... (truncated)"));
    assert!(result.contains("[end of thinking]"));
    let stripped = strip_ansi(&result);
    assert!(stripped.contains("[end of thinking]"));
}

/// render_tool_use truncation — short input is not truncated.
#[test]
fn test_render_tool_use_short_no_truncation() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "exec".into(),
        input: "ls -la".into(),
    });
    assert!(result.contains("exec"));
    assert!(result.contains("ls -la"));
    assert!(
        !result.contains("... (truncated)"),
        "short input should not be truncated"
    );
}

/// render_tool_use truncation — long input is truncated, tool name preserved.
#[test]
fn test_render_tool_use_long_truncated() {
    let renderer = TerminalRenderer::with_ansi(false);
    let long_input = "b".repeat(200);
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "exec".into(),
        input: long_input,
    });
    assert!(
        result.contains("... (truncated)"),
        "long input should be truncated"
    );
    assert!(result.contains("exec"), "tool name must be preserved");
    assert!(result.contains("⚙"));
}

/// render_tool_result truncation — regression after refactoring to truncate_to_width.
#[test]
fn test_render_tool_result_truncation_regression() {
    let renderer = TerminalRenderer::with_ansi(false);
    let short = "ok";
    let result = renderer.render_block(&ContentBlock::ToolResult {
        tool_call_id: "t1".into(),
        content: short.into(),
    });
    assert!(result.contains("ok"));
    assert!(
        !result.contains("... (truncated)"),
        "short result should not be truncated"
    );
    let width = get_terminal_width();
    let long = "d".repeat(width + 1);
    let result = renderer.render_block(&ContentBlock::ToolResult {
        tool_call_id: "t2".into(),
        content: long,
    });
    assert!(
        result.contains("... (truncated)"),
        "long result should be truncated"
    );
}

/// Truncation works correctly when ANSI mode is disabled (plain text).
#[test]
fn test_truncation_works_in_plain_text_mode() {
    let renderer = TerminalRenderer::with_ansi(false);
    let long_thinking = "c".repeat(200);
    let thinking_result = renderer.render_block(&ContentBlock::Thinking {
        thinking: long_thinking,
        signature: None,
    });
    let stripped = strip_ansi(&thinking_result);
    assert!(stripped.contains("... (truncated)"));
    assert!(stripped.contains("[end of thinking]"));
    assert!(stripped.contains("[Thinking]"));

    let long_input = "e".repeat(200);
    let tool_result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "fetch".into(),
        input: long_input,
    });
    let stripped = strip_ansi(&tool_result);
    assert!(stripped.contains("... (truncated)"));
    assert!(stripped.contains("fetch"));

    let long_content = "f".repeat(200);
    let result_result = renderer.render_block(&ContentBlock::ToolResult {
        tool_call_id: "t1".into(),
        content: long_content,
    });
    let stripped = strip_ansi(&result_result);
    assert!(stripped.contains("... (truncated)"));
}

// ── Heading bold tests (Step 1.1) ──────────────────────────────────────────

/// h1-h6 bold in ANSI mode (each level strips '#' and applies BOLD).
#[test]
fn test_heading_bold_ansi_h1_to_h6() {
    let renderer = TerminalRenderer::with_ansi(true);
    let cases = [
        ("# Title", "Title"),
        ("## Sub", "Sub"),
        ("### Section", "Section"),
        ("#### Subsection", "Subsection"),
        ("##### Detail", "Detail"),
        ("###### Smallest", "Smallest"),
    ];
    for (input, expected_content) in cases {
        let result = renderer.render_block(&ContentBlock::Text(input.into()));
        assert!(
            result.contains(expected_content),
            "h-level heading '{}' should contain '{}'",
            input,
            expected_content
        );
        assert!(
            result.contains(BOLD),
            "h-level heading '{}' should apply BOLD",
            input
        );
        assert!(
            !result.contains(input),
            "h-level heading '{}' prefix should be stripped",
            input
        );
    }
}

/// 7 '#' characters should NOT trigger heading style (not valid markdown).
#[test]
fn test_heading_h7_no_bold() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("####### H7".into()));
    // Should pass through as plain text (not a heading)
    assert!(result.contains("H7")); // passes through as plain text
}

/// Empty heading: '# ' with no content after.
#[test]
fn test_heading_empty_content() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("# ".into()));
    // Empty content after '# ' — should still wrap in BOLD
    assert!(result.contains(BOLD));
    assert!(!result.contains("# "));
}

/// No space after '#': '#Title' should not match heading pattern.
#[test]
fn test_heading_no_space_after_hash() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("#Title".into()));
    // Should be treated as plain text, '#Title' appears literally
    assert!(result.contains("#Title"));
}

/// Non-ANSI mode: h1/h3 strip '#' prefix, no ANSI codes.
#[test]
fn test_heading_no_ansi_h1_and_h3() {
    let renderer = TerminalRenderer::with_ansi(false);
    let cases = [("# Title", "Title"), ("### Sub", "Sub")];
    for (input, expected) in cases {
        let result = renderer.render_block(&ContentBlock::Text(input.into()));
        assert_eq!(result.trim(), expected);
        assert!(!result.contains(BOLD));
    }
}

/// Regression: blockquote is not affected by heading changes.
#[test]
fn test_heading_regression_blockquote() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("> quote".into()));
    assert!(result.contains("│ quote"));
    assert!(result.contains(DIM));
}

/// Regression: hr is not affected by heading changes.
#[test]
fn test_heading_regression_hr() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result = renderer.render_block(&ContentBlock::Text("---".into()));
    assert!(result.contains("───"));
    assert!(result.contains(DIM));
}

/// Regression: plain text lines are not affected by heading changes.
#[test]
fn test_heading_regression_plain_text() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::Text("just text".into()));
    assert!(result.contains("just text"));
}

/// check_line_pattern: h2/h6 bold in ANSI, h1/h2 plain in non-ANSI.
#[test]
fn test_check_line_pattern_heading_ansi_and_no_ansi() {
    let r = check_line_pattern("## Heading", true).unwrap();
    assert!(r.contains(BOLD));
    assert!(r.contains("Heading"));
    assert!(!r.contains("## Heading")); // prefix stripped

    let r = check_line_pattern("###### Tiny", true).unwrap();
    assert!(r.contains(BOLD));
    assert!(r.contains("Tiny"));

    let r = check_line_pattern("# Title", true).unwrap();
    assert!(r.contains(BOLD));
    assert!(r.contains("Title"));

    assert_eq!(check_line_pattern("# Title", false).unwrap(), "Title");
    assert_eq!(check_line_pattern("## Sub", false).unwrap(), "Sub");
}

/// check_line_pattern: plain text returns None.
#[test]
fn test_check_line_pattern_plain_none() {
    assert!(check_line_pattern("hello world", true).is_none());
}

/// check_line_pattern: h1 without trailing space returns None.
#[test]
fn test_check_line_pattern_h1_no_space() {
    assert!(check_line_pattern("#Title", true).is_none());
}

/// check_line_pattern: h7 returns None.
#[test]
fn test_check_line_pattern_h7_none() {
    assert!(check_line_pattern("####### H7", true).is_none());
}

/// check_line_pattern: hr returns DIM.
#[test]
fn test_check_line_pattern_hr() {
    let result = check_line_pattern("---", true).unwrap();
    assert!(result.contains(DIM));
    assert!(result.contains("───"));
}

/// check_line_pattern: blockquote returns DIM.
#[test]
fn test_check_line_pattern_blockquote() {
    let result = check_line_pattern("> quote", true).unwrap();
    assert!(result.contains(DIM));
    assert!(result.contains("│ quote"));
}

// ── Text block truncation tests (Step 1.2) ──────────────────────────────

/// Short text is not truncated (both ANSI and non-ANSI).
#[test]
fn test_text_block_short_no_truncation() {
    for ansi in [false, true] {
        let renderer = TerminalRenderer::with_ansi(ansi);
        let result = renderer.render_block(&ContentBlock::Text("hello".into()));
        assert!(result.contains("hello"));
        assert!(!result.contains("... (truncated)"));
    }
}

/// Long text (over terminal width) is truncated (both ANSI and non-ANSI).
#[test]
fn test_text_block_over_width_truncated() {
    for ansi in [false, true] {
        let renderer = TerminalRenderer::with_ansi(ansi);
        let long_text = "x".repeat(1000);
        let result = renderer.render_block(&ContentBlock::Text(long_text));
        assert!(result.contains("... (truncated)"));
    }
}

/// Text at terminal width - 1 is not truncated (boundary).
#[test]
fn test_text_block_at_width_no_truncation() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    // Use width - 1 chars: rendered output includes trailing \n,
    // so total is width chars which equals (not exceeds) width.
    let text_below: String = "a".repeat(width - 1);
    let result = renderer.render_block(&ContentBlock::Text(text_below));
    assert!(!result.contains("... (truncated)"));
}

/// Text exceeding terminal width is truncated (boundary).
#[test]
fn test_text_block_over_width_truncation() {
    let width = get_terminal_width();
    let renderer = TerminalRenderer::with_ansi(false);
    let over_text: String = "b".repeat(width + 1);
    let result = renderer.render_block(&ContentBlock::Text(over_text));
    assert!(result.contains("... (truncated)"));
}

/// Markdown formatted text: heading rendered before truncation.
#[test]
fn test_text_block_heading_truncated() {
    let renderer = TerminalRenderer::with_ansi(true);
    let long_heading = format!("# {}", "h".repeat(1000));
    let result = renderer.render_block(&ContentBlock::Text(long_heading));
    assert!(result.contains(BOLD)); // heading styling applied
    assert!(result.contains("... (truncated)")); // then truncated
}

/// Markdown formatted text: code block rendered before truncation.
#[test]
fn test_text_block_code_block_truncated() {
    let renderer = TerminalRenderer::with_ansi(true);
    let long_code = format!("```rust\n{}```", "c".repeat(1000));
    let result = renderer.render_block(&ContentBlock::Text(long_code));
    assert!(result.contains("... (truncated)"));
}

// ── DSL rendering order tests ───────────────────────────────────────────────
//
// Verifies that DSL preprocessing lines appear BEFORE ContentBlock output,
// matching the design doc requirement: "DSL preprocessing before ContentBlock
// traversal".

/// Normal path: DSL with Button + ContentBlocks → DSL line appears before block output.
#[test]
fn test_dsl_before_content_blocks_button() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        clean_content: String::new(),
        instructions: vec![DslInstruction::Button {
            label: "Go".into(),
            action: "navigate".into(),
            value: "url".into(),
        }],
    };
    let blocks = vec![ContentBlock::Text("body text".into())];
    let output = renderer.render(&blocks, Some(&dsl));
    let text = output.payload.as_str().unwrap();
    let dsl_pos = text
        .find("[Button:")
        .expect("DSL button line must be present");
    let block_pos = text
        .find("body text")
        .expect("ContentBlock text must be present");
    assert!(
        dsl_pos < block_pos,
        "DSL button line (pos {}) must appear before ContentBlock (pos {})",
        dsl_pos,
        block_pos
    );
}

/// Normal path: DSL with Selector + ContentBlocks → DSL line appears before block output.
#[test]
fn test_dsl_before_content_blocks_selector() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        clean_content: String::new(),
        instructions: vec![DslInstruction::Selector {
            label: "Pick".into(),
            options: vec!["x".into(), "y".into()],
            action: "choose".into(),
        }],
    };
    let blocks = vec![
        ContentBlock::Text("first".into()),
        ContentBlock::Text("second".into()),
    ];
    let output = renderer.render(&blocks, Some(&dsl));
    let text = output.payload.as_str().unwrap();
    let dsl_pos = text
        .find("[Selector:")
        .expect("DSL selector line must be present");
    let block_pos = text
        .find("first")
        .expect("ContentBlock text must be present");
    assert!(
        dsl_pos < block_pos,
        "DSL selector line (pos {}) must appear before ContentBlock (pos {})",
        dsl_pos,
        block_pos
    );
}

/// No DSL: dsl_result = None → output only contains ContentBlock rendering.
#[test]
fn test_no_dsl_content_blocks_only() {
    let renderer = TerminalRenderer::with_ansi(false);
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "run".into(),
            input: "ls".into(),
        },
    ];
    let output = renderer.render(&blocks, None);
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("hello"));
    assert!(text.contains("run"));
    assert!(!text.contains("[Button:"));
    assert!(!text.contains("[Selector:"));
}

/// Empty DSL: DslParseResult with no Button/Selector → output only contains ContentBlock rendering.
#[test]
fn test_empty_dsl_content_blocks_only() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        clean_content: String::new(),
        instructions: vec![],
    };
    let blocks = vec![ContentBlock::Text("content here".into())];
    let output = renderer.render(&blocks, Some(&dsl));
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("content here"));
    assert!(!text.contains("[Button:"));
    assert!(!text.contains("[Selector:"));
}

/// Boundary: empty ContentBlocks + DSL → output only contains DSL hint lines.
#[test]
fn test_empty_blocks_with_dsl() {
    let renderer = TerminalRenderer::with_ansi(false);
    let dsl = DslParseResult {
        clean_content: String::new(),
        instructions: vec![DslInstruction::Button {
            label: "Submit".into(),
            action: "confirm".into(),
            value: "yes".into(),
        }],
    };
    let output = renderer.render(&[], Some(&dsl));
    let text = output.payload.as_str().unwrap();
    assert!(text.contains("[Button:"), "DSL button must be present");
    assert!(text.contains("Submit"));
    assert!(text.contains("confirm"));
}

// ── ToolUse format tests (Step 1.3) ───────────────────────────────────────
/// ToolUse renders ⚙ tool_name(input) — plain text and ANSI modes.
#[test]
fn test_tool_use_format_normal_and_empty() {
    for ansi in [false, true] {
        let renderer = TerminalRenderer::with_ansi(ansi);
        // Normal: JSON input inside parentheses
        let result = renderer.render_block(&ContentBlock::ToolUse {
            id: "t1".into(),
            name: "exec".into(),
            input: r#"{"cmd":"ls"}"#.into(),
        });
        assert!(result.contains("⚙"));
        assert!(result.contains("exec"));
        assert!(result.contains(r#"{"cmd":"ls"}"#));
        assert!(
            !result.contains("{}"),
            "should not use bare braces as wrapper"
        );
        if ansi {
            assert!(result.contains(DIM));
            assert!(result.contains(BOLD));
            assert!(result.contains(CYAN));
        }
        // Empty input: parentheses with empty content
        let result = renderer.render_block(&ContentBlock::ToolUse {
            id: "t2".into(),
            name: "noop".into(),
            input: String::new(),
        });
        let stripped = strip_ansi(&result);
        assert!(stripped.contains("()"));
    }
}
/// ToolUse with JSON object/array input renders correctly.
#[test]
fn test_tool_use_format_json_input() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t1".into(),
        name: "write".into(),
        input: r#"{"path":"/tmp/a.txt","content":"hello"}"#.into(),
    });
    assert!(result.contains("⚙"));
    assert!(result.contains("write"));
    assert!(result.contains("/tmp/a.txt"));
    let result = renderer.render_block(&ContentBlock::ToolUse {
        id: "t2".into(),
        name: "batch".into(),
        input: r#"[{"a":1},{"b":2}]"#.into(),
    });
    assert!(result.contains("⚙"));
    assert!(result.contains("batch"));
    assert!(result.contains(r#"{"a":1}"#));
    assert!(result.contains(r#"{"b":2}"#));
}
