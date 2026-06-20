//! Terminal channel — renderer and adapter for CLI chat.
//!
//! - [`TerminalRenderer`]: converts LLM [`ContentBlock`]s to ANSI-formatted
//!   text. Detects terminal ANSI capability and falls back to plain text.
//! - [`TerminalAdapter`]: reads user input from stdin, producing
//!   [`NormalizedMessage`]s with blank-line-delimited message boundaries.

use crate::im_adapter::code_block::{parse_content_segments, ContentSegment};
use crate::im_adapter::normalized::NormalizedMessage;
use crate::im_adapter::plugin::IMPlugin;
use crate::im_adapter::renderer::{RenderedOutput, Renderer};
use crate::im_adapter::AdapterError;
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;
use async_trait::async_trait;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// ANSI escape codes
// ---------------------------------------------------------------------------

/// ANSI style: bold
pub(crate) const BOLD: &str = "\x1b[1m";
/// ANSI style: dim
pub(crate) const DIM: &str = "\x1b[2m";
/// ANSI style: cyan
pub(crate) const CYAN: &str = "\x1b[36m";
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

/// Format a single line of markdown text.
/// `ansi=true` applies ANSI styling; `ansi=false` strips markdown markers.
fn format_line(line: &str, ansi: bool) -> String {
    if let Some(styled) = check_line_pattern(line, ansi) {
        return styled;
    }
    let chars: Vec<char> = line.chars().collect();
    let spans = parse_inline_spans(&chars);
    apply_inline_styling(&spans, ansi)
}

/// Check line-level patterns (heading, blockquote, hr).
/// Returns formatted string if matched, None otherwise.
fn check_line_pattern(line: &str, ansi: bool) -> Option<String> {
    if let Some(rest) = line.strip_prefix("# ") {
        return Some(if ansi {
            format!("{}{}{}", BOLD, rest, RESET)
        } else {
            rest.to_string()
        });
    }
    if line.trim() == "---" {
        return Some(if ansi {
            format!("{}───{}", DIM, RESET)
        } else {
            "───".to_string()
        });
    }
    if let Some(rest) = line.strip_prefix("> ") {
        return Some(if ansi {
            format!("{}│ {}{}", DIM, rest, RESET)
        } else {
            format!("│ {}", rest)
        });
    }
    None
}

/// Apply ANSI code around `text` when `ansi` is true.
fn apply_inline_styled_text(text: &str, ansi_code: &str, ansi: bool) -> String {
    let mut out = String::new();
    if ansi {
        out.push_str(ansi_code);
    }
    out.push_str(text);
    if ansi {
        out.push_str(RESET);
    }
    out
}

/// Generic inline styling: applies ANSI codes or strips markdown markers.
fn apply_inline_styling(spans: &[InlineSpan], ansi: bool) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            InlineSpan::Bold(text) => {
                out.push_str(&apply_inline_styled_text(text, BOLD, ansi));
            }
            InlineSpan::Italic(text) => {
                out.push_str(&apply_inline_styled_text(text, ITALIC, ansi));
            }
            InlineSpan::Code(text) => {
                out.push_str(&apply_inline_styled_text(text, BOLD, ansi));
            }
            InlineSpan::Link { text, url } => {
                if ansi {
                    out.push_str(&format!("{}{}{} ({})", BOLD, text, RESET, url));
                } else {
                    out.push_str(&format!("{} ({})", text, url));
                }
            }
            InlineSpan::Text(text) => {
                out.push_str(text);
            }
        }
    }
    out
}

/// Parsed inline markdown spans.
enum InlineSpan {
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, url: String },
    Text(String),
}

/// Extract bold spans from chars starting at position.
/// Returns (end_pos, Some(text)) if found, or (start, None).
fn extract_bold(chars: &[char], start: usize) -> (usize, Option<String>) {
    if start + 1 < chars.len() && chars[start] == '*' && chars[start + 1] == '*' {
        if let Some(end) = find_double_star_end(chars, start + 2) {
            let text: String = chars[start + 2..end].iter().collect();
            return (end + 2, Some(text));
        }
    }
    (start, None)
}

/// Extract italic spans from chars starting at position.
fn extract_italic(chars: &[char], start: usize) -> (usize, Option<String>) {
    if chars[start] == '*' && (start + 1 >= chars.len() || chars[start + 1] != '*') {
        if let Some(end) = find_single_star_end(chars, start + 1) {
            let text: String = chars[start + 1..end].iter().collect();
            return (end + 1, Some(text));
        }
    }
    (start, None)
}

/// Extract inline code spans from chars starting at position.
fn extract_code(chars: &[char], start: usize) -> (usize, Option<String>) {
    if chars[start] == '`' {
        if let Some(end) = find_char_end(chars, start + 1, '`') {
            let text: String = chars[start + 1..end].iter().collect();
            return (end + 1, Some(text));
        }
    }
    (start, None)
}

/// Extract link spans from chars starting at position.
fn extract_link(chars: &[char], start: usize) -> (usize, Option<(String, String)>) {
    if chars[start] == '[' {
        let len = chars.len();
        if let Some(bracket_end) = find_char_end(chars, start + 1, ']') {
            if bracket_end + 1 < len && chars[bracket_end + 1] == '(' {
                if let Some(paren_end) = find_char_end(chars, bracket_end + 2, ')') {
                    let text: String = chars[start + 1..bracket_end].iter().collect();
                    let url: String = chars[bracket_end + 2..paren_end].iter().collect();
                    return (paren_end + 1, Some((text, url)));
                }
            }
        }
    }
    (start, None)
}

/// Push accumulated plain text into spans if non-empty.
fn flush_text(spans: &mut Vec<InlineSpan>, chars: &[char], start: usize, end: usize) {
    if start < end {
        let text: String = chars[start..end].iter().collect();
        spans.push(InlineSpan::Text(text));
    }
}

/// Parse inline markdown patterns into spans.
fn parse_inline_spans(chars: &[char]) -> Vec<InlineSpan> {
    let mut spans = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    let len = chars.len();

    while i < len {
        let (new_i, bold_text) = extract_bold(chars, i);
        if let Some(text) = bold_text {
            flush_text(&mut spans, chars, text_start, i);
            spans.push(InlineSpan::Bold(text));
            i = new_i;
            text_start = i;
            continue;
        }
        let (new_i, italic_text) = extract_italic(chars, i);
        if let Some(text) = italic_text {
            flush_text(&mut spans, chars, text_start, i);
            spans.push(InlineSpan::Italic(text));
            i = new_i;
            text_start = i;
            continue;
        }
        let (new_i, code_text) = extract_code(chars, i);
        if let Some(text) = code_text {
            flush_text(&mut spans, chars, text_start, i);
            spans.push(InlineSpan::Code(text));
            i = new_i;
            text_start = i;
            continue;
        }
        let (new_i, link_opt) = extract_link(chars, i);
        if let Some((text, url)) = link_opt {
            flush_text(&mut spans, chars, text_start, i);
            spans.push(InlineSpan::Link { text, url });
            i = new_i;
            text_start = i;
            continue;
        }
        i += 1;
    }
    flush_text(&mut spans, chars, text_start, len);
    spans
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

/// Render a fenced code block with language annotation, line numbers,
/// and optional syntax highlighting.
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
            let highlighted = highlight_line(line, language);
            out.push_str(&format!("{} {} │ {}{}", DIM, num, RESET, highlighted));
        } else {
            out.push_str(&format!("  {} │ {}", num, line));
        }
        out.push('\n');
    }

    out
}

/// ANSI color codes for syntax highlighting.
const ANSI_KEYWORD: &str = "\x1b[35m";
const ANSI_COMMENT: &str = "\x1b[90m";
/// Highlight a string literal in source code.
fn highlight_string(chars: &[char], start: usize, out: &mut String) -> usize {
    let q = chars[start];
    out.push(q);
    let mut i = start + 1;
    while i < chars.len() && chars[i] != q {
        if chars[i] == '\\' {
            out.push(chars[i]);
            i += 1;
            if i < chars.len() {
                out.push(chars[i]);
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    if i < chars.len() {
        out.push(chars[i]);
        i += 1;
    }
    i
}

/// Highlight a word — check for keywords and wrap in ANSI color.
fn highlight_word(chars: &[char], start: usize, language: &str, out: &mut String) -> usize {
    let mut i = start;
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    let word: String = chars[start..i].iter().collect();
    if is_keyword(&word, language) {
        out.push_str(ANSI_KEYWORD);
        out.push_str(&word);
        out.push_str(RESET);
    } else {
        out.push_str(&word);
    }
    i
}

/// Highlight keywords, strings, and comments in a code line.
fn highlight_line(line: &str, language: &str) -> String {
    let trimmed = line.trim_start();
    if is_comment(trimmed, language) {
        return format!("{}{}{}", ANSI_COMMENT, line, RESET);
    }
    let mut out = String::with_capacity(line.len() * 2);
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i] == '"' || chars[i] == '\'' {
            i = highlight_string(&chars, i, &mut out);
        } else if chars[i].is_alphabetic() || chars[i] == '_' {
            i = highlight_word(&chars, i, language, &mut out);
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Check if a trimmed line is a comment.
fn is_comment(trimmed: &str, language: &str) -> bool {
    matches!(
        language,
        "python" | "ruby" | "bash" | "sh" | "yaml" | "toml"
    ) && trimmed.starts_with('#')
        || (matches!(
            language,
            "rust" | "go" | "javascript" | "typescript" | "c" | "cpp" | "java"
        ) && (trimmed.starts_with("//") || trimmed.starts_with("/*")))
        || (matches!(language, "haskell" | "lua") && trimmed.starts_with("--"))
}

/// Check if a word is a keyword for the given language.
fn is_keyword(word: &str, language: &str) -> bool {
    match language {
        "rust" | "" => is_rust_keyword(word),
        "python" => is_python_keyword(word),
        "javascript" | "typescript" => is_js_keyword(word),
        "go" => is_go_keyword(word),
        "c" | "cpp" | "java" => is_c_keyword(word),
        _ => false,
    }
}

fn is_rust_keyword(word: &str) -> bool {
    matches!(
        word,
        "fn" | "let"
            | "mut"
            | "pub"
            | "mod"
            | "use"
            | "struct"
            | "enum"
            | "impl"
            | "trait"
            | "match"
            | "if"
            | "else"
            | "for"
            | "while"
            | "loop"
            | "return"
            | "self"
            | "Self"
            | "super"
            | "crate"
            | "async"
            | "await"
            | "move"
            | "ref"
            | "type"
            | "where"
    )
}

fn is_python_keyword(word: &str) -> bool {
    matches!(
        word,
        "def"
            | "class"
            | "import"
            | "from"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "try"
            | "except"
            | "with"
            | "as"
            | "in"
            | "not"
            | "and"
            | "or"
            | "lambda"
            | "yield"
            | "print"
    )
}

fn is_js_keyword(word: &str) -> bool {
    matches!(
        word,
        "function"
            | "const"
            | "let"
            | "var"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "class"
            | "new"
            | "this"
            | "import"
            | "export"
            | "default"
            | "switch"
            | "case"
            | "break"
    )
}

fn is_go_keyword(word: &str) -> bool {
    matches!(
        word,
        "func"
            | "package"
            | "import"
            | "return"
            | "if"
            | "else"
            | "for"
            | "range"
            | "struct"
            | "interface"
            | "type"
            | "var"
            | "const"
            | "defer"
            | "go"
            | "select"
            | "chan"
            | "map"
    )
}

fn is_c_keyword(word: &str) -> bool {
    matches!(
        word,
        "int"
            | "float"
            | "double"
            | "char"
            | "void"
            | "if"
            | "else"
            | "for"
            | "while"
            | "return"
            | "class"
            | "public"
            | "private"
            | "static"
            | "new"
            | "this"
            | "switch"
            | "case"
            | "break"
    )
}

// ---------------------------------------------------------------------------
// Markdown → ANSI block rendering
// ---------------------------------------------------------------------------

/// Convert markdown content to text, with optional ANSI styling.
fn render_markdown(content: &str, ansi: bool) -> String {
    let segments = parse_content_segments(content);
    let mut out = String::new();

    for seg in &segments {
        match seg {
            ContentSegment::Hr => {
                if ansi {
                    out.push_str(&format!("{}───{}\n\n", DIM, RESET));
                } else {
                    out.push_str("───\n\n");
                }
            }
            ContentSegment::CodeBlock { language, code } => {
                out.push('\n');
                out.push_str(&render_code_block(language, code, ansi));
                out.push('\n');
            }
            ContentSegment::Markdown(line) => {
                out.push_str(&format_line(line, ansi));
                out.push('\n');
            }
        }
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
            ContentBlock::Text(text) => render_markdown(text, self.ansi),
            ContentBlock::Thinking(text) => self.render_thinking(text),
            ContentBlock::ToolUse { name, input, .. } => self.render_tool_use(name, input),
            ContentBlock::ToolResult { content, .. } => self.render_tool_result(content),
            ContentBlock::Image(name) => self.render_placeholder("image", name),
            ContentBlock::Audio(name) => self.render_placeholder("audio", name),
            ContentBlock::File(name) => self.render_placeholder("file", name),
        }
    }

    /// Render a placeholder for unsupported content types.
    fn render_placeholder(&self, kind: &str, name: &str) -> String {
        if self.ansi {
            format!("{}[{}: {}]{}\n", DIM, kind, name, RESET)
        } else {
            format!("[{}: {}]\n", kind, name)
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
                "{}⚙ {}{}{}{}({}{}){}{}\n",
                DIM, BOLD, CYAN, name, RESET, DIM, input, DIM, RESET
            )
        } else {
            format!("⚙ {}({})\n", name, input)
        }
    }

    /// Render a ToolResult block, truncating at terminal width.
    fn render_tool_result(&self, content: &str) -> String {
        let max_width = get_terminal_width();
        let display = if content.chars().count() > max_width {
            let truncated: String = content.chars().take(max_width).collect();
            format!("{}... (truncated)", truncated)
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
        let text = if self.ansi {
            output_text
        } else {
            strip_ansi(&output_text)
        };
        let payload = serde_json::json!({
            "content": {
                "text": text
            }
        });

        RenderedOutput {
            msg_type: "text".into(),
            payload,
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalAdapter
// ---------------------------------------------------------------------------

/// Adapter that reads user input from stdin and produces
/// [`NormalizedMessage`]s.
///
/// Messages are delimited by blank lines: a sequence of non-empty lines
/// followed by an empty line forms one message. Trailing newlines within
/// a message are preserved.
#[derive(Debug, Clone, Default)]
pub struct TerminalAdapter;

impl TerminalAdapter {
    /// Create a new adapter.
    pub fn new() -> Self {
        Self
    }

    /// Read a single message from stdin.
    ///
    /// Collects lines until a blank line is encountered, then returns the
    /// joined content as a [`NormalizedMessage`]. Returns `None` when the
    /// input is empty or stdin is exhausted (EOF).
    pub fn read_input(&self) -> Option<NormalizedMessage> {
        let stdin = io::stdin();
        let mut lines = Vec::new();

        for line in stdin.lock().lines() {
            match line {
                Ok(text) => {
                    if text.trim().is_empty() {
                        // Blank line → message boundary
                        if lines.is_empty() {
                            continue;
                        }
                        break;
                    }
                    lines.push(text);
                }
                Err(_) => break,
            }
        }

        if lines.is_empty() {
            return None;
        }

        let content = lines.join("\n");
        Some(self.make_message(content))
    }

    /// Build a [`NormalizedMessage`] from raw text content.
    fn make_message(&self, content: String) -> NormalizedMessage {
        NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: current_uid(),
            peer_id: "cli".to_string(),
            content,
            timestamp: current_timestamp(),
            thread_id: None,
            account_id: None,
        }
    }
}

/// Return the terminal width in columns, falling back to 120.
fn get_terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(120)
}

/// Return the current user's system UID as a string.
///
/// On Unix, uses `libc::getuid()`. On other platforms, falls back to a
/// static identifier.
pub(crate) fn current_uid() -> String {
    #[cfg(unix)]
    {
        // SAFETY: libc::getuid() is always safe — it reads the calling
        // process's real user ID without side effects.
        unsafe { libc::getuid() }.to_string()
    }
    #[cfg(not(unix))]
    {
        "terminal-user".to_string()
    }
}

/// Return the current Unix timestamp in seconds.
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// TerminalPlugin
// ---------------------------------------------------------------------------

/// Unified IM plugin for the terminal (CLI) channel.
///
/// Wraps [`TerminalAdapter`] (stdin input) and [`TerminalRenderer`] (ANSI
/// output) behind the [`IMPlugin`] trait so the Gateway can route messages
/// through the terminal channel just like any other platform.
pub struct TerminalPlugin {
    adapter: Arc<TerminalAdapter>,
    renderer: Arc<TerminalRenderer>,
}

impl TerminalPlugin {
    /// Create a new terminal plugin with auto-detected ANSI capability.
    pub fn new() -> Self {
        Self {
            adapter: Arc::new(TerminalAdapter::new()),
            renderer: Arc::new(TerminalRenderer::new()),
        }
    }

    /// Create a terminal plugin with explicit ANSI mode.
    ///
    /// Useful for testing or forcing a specific output mode.
    pub fn with_ansi(ansi: bool) -> Self {
        Self {
            adapter: Arc::new(TerminalAdapter::new()),
            renderer: Arc::new(TerminalRenderer::with_ansi(ansi)),
        }
    }
}

impl Default for TerminalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IMPlugin for TerminalPlugin {
    fn platform(&self) -> &str {
        "terminal"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        let adapter = self.adapter.clone();
        let result = tokio::task::spawn_blocking(move || adapter.read_input())
            .await
            .map_err(|e| AdapterError::InvalidPayload(e.to_string()))?;
        Ok(result)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        self.renderer.render(content_blocks, dsl_result)
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        let mut stdout = io::stdout();
        Write::write_all(&mut stdout, text.as_bytes())
            .map_err(|e| AdapterError::SendFailed(e.to_string()))?;
        Write::flush(&mut stdout).map_err(|e| AdapterError::SendFailed(e.to_string()))?;
        Ok(())
    }
}
