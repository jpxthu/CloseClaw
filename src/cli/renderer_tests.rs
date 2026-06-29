//! Unit tests for the TerminalRenderer component.
//!
//! Covers direct rendering of content blocks, DSL elements, markdown,
//! code blocks, ANSI helpers, and edge cases.

use crate::cli::renderer::{strip_ansi, TerminalRenderer, BOLD, CYAN, DIM, ITALIC};
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
    let text = output
        .payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap();
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
    let text = output
        .payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap();
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
    let text = output
        .payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap();
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
    let text = output
        .payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap();
    assert!(
        !text.contains("[Button:"),
        "DSL button should NOT appear in output"
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
    let text = output
        .payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap();
    assert!(
        !text.contains("[Selector:"),
        "DSL selector should NOT appear in output"
    );
}

#[test]
fn test_render_ansi_strips_codes_when_disabled() {
    let renderer = TerminalRenderer::with_ansi(false);
    let output = renderer.render(&[ContentBlock::Text("**bold**".into())], None);
    let text = output
        .payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap();
    assert!(!text.contains(BOLD), "ansi should not contain BOLD escape");
}

#[test]
fn test_render_ansi_contains_codes_when_enabled() {
    let renderer = TerminalRenderer::with_ansi(true);
    let output = renderer.render(&[ContentBlock::Text("**bold**".into())], None);
    let text = output
        .payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap();
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
