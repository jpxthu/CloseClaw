//! Terminal renderer — converts LLM [`ContentBlock`]s to ANSI-formatted text.
//!
//! Detects terminal ANSI capability and falls back to plain text when
//! unsupported. Handles all block types defined in the design doc:
//! Text, Thinking, ToolUse, ToolResult, and unsupported placeholders.

use crate::im_adapter::code_block::{parse_content_segments, ContentSegment};
use crate::im_adapter::renderer::{RenderedOutput, Renderer};
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;

// ---------------------------------------------------------------------------
// ANSI escape codes
// ---------------------------------------------------------------------------

/// ANSI style: bold
pub(crate) const BOLD: &str = "\x1b[1m";
/// ANSI style: dim
pub(crate) const DIM: &str = "\x1b[2m";
/// ANSI style: italic
pub(crate) const ITALIC: &str = "\x1b[3m";
/// ANSI reset all styles
pub(crate) const RESET: &str = "\x1b[0m";

// ---------------------------------------------------------------------------
// ANSI capability detection
// ---------------------------------------------------------------------------

/// Returns `true` if the current terminal supports ANSI escape sequences.
///
/// Checks the `TERM` environment variable for known ANSI-capable values
/// (`xterm`, `screen`, `ansi`, `vt100`, `color`). On Windows, detects the
/// Windows Terminal environment via `WT_SESSION`.
fn supports_ansi() -> bool {
    if std::env::var("WT_SESSION").is_ok() {
        return true;
    }
    std::env::var("TERM")
        .map(|term| {
            let t = term.to_lowercase();
            t.contains("xterm")
                || t.contains("screen")
                || t.contains("ansi")
                || t.contains("vt100")
                || t.contains("color")
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Plain-text fallback
// ---------------------------------------------------------------------------

/// Strip all ANSI escape sequences from `text`.
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until a control character (the final byte of the sequence)
            for ch in chars.by_ref() {
                if ch.is_ascii_alphabetic() || ch == 'm' {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Markdown → ANSI inline conversion
// ---------------------------------------------------------------------------

/// Apply inline ANSI formatting to a single line of markdown text.
fn ansi_format_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len() * 2);
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Headings: lines starting with `# `
        if i == 0 && line.starts_with("# ") {
            out.push_str(BOLD);
            out.push_str(&line[2..]);
            out.push_str(RESET);
            return out;
        }

        // Horizontal rule
        if i == 0 && line.trim() == "---" {
            return format!("{}───{}", DIM, RESET);
        }

        // Bold: **text**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_double_star_end(&chars, i + 2) {
                out.push_str(BOLD);
                for c in &chars[i + 2..end] {
                    out.push(*c);
                }
                out.push_str(RESET);
                i = end + 2;
                continue;
            }
        }

        // Italic: *text*
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if let Some(end) = find_single_star_end(&chars, i + 1) {
                out.push_str(ITALIC);
                for c in &chars[i + 1..end] {
                    out.push(*c);
                }
                out.push_str(RESET);
                i = end + 1;
                continue;
            }
        }

        // Inline code: `text`
        if chars[i] == '`' {
            if let Some(end) = find_char_end(&chars, i + 1, '`') {
                let code_text: String = chars[i + 1..end].iter().collect();
                out.push_str(BOLD);
                out.push_str(&code_text);
                out.push_str(RESET);
                i = end + 1;
                continue;
            }
        }

        // Link: [text](url) → "text (url)"
        if chars[i] == '[' {
            if let Some(bracket_end) = find_char_end(&chars, i + 1, ']') {
                if bracket_end + 1 < len && chars[bracket_end + 1] == '(' {
                    if let Some(paren_end) = find_char_end(&chars, bracket_end + 2, ')') {
                        let link_text: String = chars[i + 1..bracket_end].iter().collect();
                        let link_url: String = chars[bracket_end + 2..paren_end].iter().collect();
                        out.push_str(&format!("{}{}{} ({})", BOLD, link_text, RESET, link_url));
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }

        // Blockquote: > text
        if i == 0 && line.starts_with("> ") {
            out.push_str(DIM);
            out.push_str("│ ");
            out.push_str(&line[2..]);
            out.push_str(RESET);
            return out;
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

/// Find the closing `**` for bold starting at `start`.
fn find_double_star_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == '*' && chars[i + 1] == '*' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find the closing `*` for italic starting at `start`, skipping `**`.
fn find_single_star_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '*' {
            // Skip over `**` so bold is handled elsewhere
            if i + 1 < chars.len() && chars[i + 1] == '*' {
                i += 2;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find closing `ch` starting at `start`, not crossing newlines.
fn find_char_end(chars: &[char], start: usize, ch: char) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == ch {
            return Some(i);
        }
        if chars[i] == '\n' {
            return None;
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Code block rendering
// ---------------------------------------------------------------------------

/// Render a fenced code block with language annotation and line numbers.
fn render_code_block(language: &str, code: &str, ansi: bool) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let line_width = format!("{}", lines.len()).len();
    let mut out = String::new();

    if ansi && !language.is_empty() {
        out.push_str(&format!("{} {} {}\n", DIM, language, RESET));
    } else if !language.is_empty() {
        out.push_str(&format!("  {}\n", language));
    }

    for (i, line) in lines.iter().enumerate() {
        let num = format!("{:>width$}", i + 1, width = line_width);
        if ansi {
            out.push_str(&format!("{} {} │ {}{}", DIM, num, RESET, line));
        } else {
            out.push_str(&format!("  {} │ {}", num, line));
        }
        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// Markdown → ANSI block rendering
// ---------------------------------------------------------------------------

/// Convert markdown content to ANSI-formatted text.
fn render_markdown_ansi(content: &str) -> String {
    let segments = parse_content_segments(content);
    let mut out = String::new();

    for seg in &segments {
        match seg {
            ContentSegment::Hr => {
                out.push_str(&format!("{}───{}\n\n", DIM, RESET));
            }
            ContentSegment::CodeBlock { language, code } => {
                out.push('\n');
                out.push_str(&render_code_block(language, code, true));
                out.push('\n');
            }
            ContentSegment::Markdown(line) => {
                out.push_str(&ansi_format_inline(line));
                out.push('\n');
            }
        }
    }

    out
}

/// Convert markdown content to plain text (no ANSI).
fn render_markdown_plain(content: &str) -> String {
    let segments = parse_content_segments(content);
    let mut out = String::new();

    for seg in &segments {
        match seg {
            ContentSegment::Hr => {
                out.push_str("───\n\n");
            }
            ContentSegment::CodeBlock { language, code } => {
                out.push('\n');
                out.push_str(&render_code_block(language, code, false));
                out.push('\n');
            }
            ContentSegment::Markdown(line) => {
                // Strip markdown formatting markers for plain text
                let stripped = strip_markdown(line);
                out.push_str(&stripped);
                out.push('\n');
            }
        }
    }

    out
}

/// Remove common markdown formatting markers for plain-text output.
fn strip_markdown(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Heading markers
        if i == 0 && line.starts_with("# ") {
            out.push_str(&line[2..]);
            return out;
        }

        // Bold: **text** → text
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_double_star_end(&chars, i + 2) {
                for c in &chars[i + 2..end] {
                    out.push(*c);
                }
                i = end + 2;
                continue;
            }
        }

        // Italic: *text* → text
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if let Some(end) = find_single_star_end(&chars, i + 1) {
                for c in &chars[i + 1..end] {
                    out.push(*c);
                }
                i = end + 1;
                continue;
            }
        }

        // Inline code: `text` → text
        if chars[i] == '`' {
            if let Some(end) = find_char_end(&chars, i + 1, '`') {
                for c in &chars[i + 1..end] {
                    out.push(*c);
                }
                i = end + 1;
                continue;
            }
        }

        // Link: [text](url) → text (url)
        if chars[i] == '[' {
            if let Some(bracket_end) = find_char_end(&chars, i + 1, ']') {
                if bracket_end + 1 < len && chars[bracket_end + 1] == '(' {
                    if let Some(paren_end) = find_char_end(&chars, bracket_end + 2, ')') {
                        let link_text: String = chars[i + 1..bracket_end].iter().collect();
                        let link_url: String = chars[bracket_end + 2..paren_end].iter().collect();
                        out.push_str(&format!("{} ({})", link_text, link_url));
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }

        // Blockquote: > text → │ text
        if i == 0 && line.starts_with("> ") {
            out.push_str("│ ");
            out.push_str(&line[2..]);
            return out;
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// TerminalRenderer
// ---------------------------------------------------------------------------

/// Renderer for the terminal (CLI) channel.
///
/// Converts LLM [`ContentBlock`]s into ANSI-formatted text suitable for
/// stdout display. Detects terminal ANSI support and falls back to plain
/// text when unavailable.
#[derive(Debug, Clone)]
pub struct TerminalRenderer {
    ansi: bool,
}

impl Default for TerminalRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalRenderer {
    /// Create a new renderer that auto-detects ANSI capability.
    pub fn new() -> Self {
        Self {
            ansi: supports_ansi(),
        }
    }

    /// Create a renderer with explicit ANSI mode.
    ///
    /// Useful for testing or forcing a specific output mode.
    pub fn with_ansi(ansi: bool) -> Self {
        Self { ansi }
    }

    /// Render a single [`ContentBlock`] to text.
    fn render_block(&self, block: &ContentBlock) -> String {
        match block {
            ContentBlock::Text(text) => {
                if self.ansi {
                    render_markdown_ansi(text)
                } else {
                    render_markdown_plain(text)
                }
            }
            ContentBlock::Thinking(text) => self.render_thinking(text),
            ContentBlock::ToolUse { name, input, .. } => self.render_tool_use(name, input),
            ContentBlock::ToolResult { content, .. } => self.render_tool_result(content),
        }
    }

    /// Render a Thinking block with boundary markers.
    fn render_thinking(&self, text: &str) -> String {
        if self.ansi {
            format!(
                "{}[Thinking]{}\n  {}{}{}[end of thinking]{}\n",
                DIM, RESET, DIM, text, DIM, RESET
            )
        } else {
            format!("[Thinking]\n  {}\n[end of thinking]\n", text)
        }
    }

    /// Render a ToolUse block: `⚙ tool_name(args)`.
    fn render_tool_use(&self, name: &str, input: &str) -> String {
        if self.ansi {
            format!(
                "{}⚙ {}{}{}({}{}){}{}\n",
                DIM, BOLD, name, RESET, DIM, input, DIM, RESET
            )
        } else {
            format!("⚙ {}({})\n", name, input)
        }
    }

    /// Render a ToolResult block, truncating at terminal width.
    fn render_tool_result(&self, content: &str) -> String {
        // Truncate at 120 characters with indicator
        let max_width = 120;
        let display = if content.len() > max_width {
            format!("{}... (truncated)", &content[..max_width])
        } else {
            content.to_string()
        };

        if self.ansi {
            format!("{}{}{}\n", DIM, display, RESET)
        } else {
            format!("{}\n", display)
        }
    }

    /// Render DSL instructions as plain text prompts.
    fn render_dsl(&self, dsl_result: &DslParseResult) -> String {
        let mut out = String::new();
        for inst in &dsl_result.instructions {
            let label = match inst {
                crate::processor_chain::dsl_parser::DslInstruction::Button { label, .. } => label,
            };
            out.push_str(&format!("[Button: {}]\n", label));
        }
        out
    }
}

impl Renderer for TerminalRenderer {
    fn platform(&self) -> &str {
        "terminal"
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let mut output_text = String::new();

        for block in content_blocks {
            let rendered = self.render_block(block);
            output_text.push_str(&rendered);
            output_text.push('\n');
        }

        // Append DSL prompts if present
        if let Some(dsl) = dsl_result {
            let dsl_text = self.render_dsl(dsl);
            if !dsl_text.is_empty() {
                output_text.push_str(&dsl_text);
            }
        }

        // Build the payload — the text content for stdout
        let payload = serde_json::json!({
            "content": {
                "text": if self.ansi { output_text.clone() } else { strip_ansi(&output_text) }
            }
        });

        RenderedOutput {
            msg_type: "text".into(),
            payload,
        }
    }
}
