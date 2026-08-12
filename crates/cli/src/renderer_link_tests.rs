//! Link rendering tests for the TerminalRenderer component.
//!
//! Covers link formatting in ANSI and non-ANSI modes, inline links,
//! multiple links, special URL characters, and render_block integration.

use crate::renderer::{format_line, TerminalRenderer, BOLD, ITALIC};
use closeclaw_llm::types::ContentBlock;

// ── Link rendering tests ───────────────────────────────────────────────────

/// Link in ANSI mode: plain text output `text (url)`, no BOLD/RESET escapes.
#[test]
fn test_link_ansi_no_bold() {
    let result = format_line("[example](https://example.com)", true);
    assert_eq!(result, "example (https://example.com)");
    assert!(
        !result.contains(BOLD),
        "link text must not be wrapped in BOLD"
    );
    assert!(
        !result.contains(ITALIC),
        "link text must not be wrapped in ITALIC"
    );
}

/// Link in non-ANSI mode: plain text output `text (url)`.
#[test]
fn test_link_no_ansi() {
    let result = format_line("[example](https://example.com)", false);
    assert_eq!(result, "example (https://example.com)");
}

/// Both ANSI and non-ANSI produce identical output for links.
#[test]
fn test_link_ansi_and_no_ansi_identical() {
    let input = "[Rust](https://rust-lang.org)";
    let ansi_result = format_line(input, true);
    let plain_result = format_line(input, false);
    assert_eq!(ansi_result, plain_result);
    assert_eq!(ansi_result, "Rust (https://rust-lang.org)");
}

/// Link text and URL are both correctly present in the output.
#[test]
fn test_link_text_and_url_present() {
    let result = format_line("[click here](https://example.com/path?q=1)", true);
    assert!(
        result.starts_with("click here"),
        "text must appear at start"
    );
    assert!(
        result.contains("https://example.com/path?q=1"),
        "URL must appear in output"
    );
    assert!(
        result.contains("(https://example.com/path?q=1)"),
        "URL must be wrapped in parentheses"
    );
}

/// Link embedded in surrounding text preserves context.
#[test]
fn test_link_inline_with_surrounding_text() {
    let result = format_line("see [here](https://x.com) for details", true);
    assert!(result.contains("see "));
    assert!(result.contains("here (https://x.com)"));
    assert!(result.contains(" for details"));
}

/// Link with special characters in URL.
#[test]
fn test_link_special_url_chars() {
    let result = format_line("[doc](https://example.com/a?b=1&c=2#top)", false);
    assert_eq!(result, "doc (https://example.com/a?b=1&c=2#top)");
}

/// Multiple links on one line.
#[test]
fn test_link_multiple_on_line() {
    let result = format_line("[a](http://a.com) and [b](http://b.com)", true);
    assert_eq!(result, "a (http://a.com) and b (http://b.com)");
}

/// Link via render_block in ANSI mode: verify no BOLD around link text.
#[test]
fn test_link_render_block_ansi_no_bold() {
    let renderer = TerminalRenderer::with_ansi(true);
    let result =
        renderer.render_block(&ContentBlock::Text("[example](https://example.com)".into()));
    assert!(
        !result.contains(BOLD),
        "link text must not be BOLD in ANSI mode"
    );
    assert!(result.contains("example"));
    assert!(result.contains("https://example.com"));
}

/// Link via render_block in non-ANSI mode.
#[test]
fn test_link_render_block_no_ansi() {
    let renderer = TerminalRenderer::with_ansi(false);
    let result =
        renderer.render_block(&ContentBlock::Text("[example](https://example.com)".into()));
    assert!(result.contains("example (https://example.com)"));
}
