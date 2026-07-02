//! Terminal Renderer — ANSI-aware outbound rendering component.
//!
//! [`TerminalRenderer`] converts [`ContentBlock`]s and DSL parse results
//! into ANSI-formatted text for stdout. It handles markdown, code blocks,
//! thinking blocks, tool use/result, and DSL element rendering.

use closeclaw_common::processor::DslParseResult;
use closeclaw_im_adapter::code_block::{parse_content_segments, ContentSegment};
use closeclaw_im_adapter::plugin::RenderedOutput;
use closeclaw_im_adapter::streaming::DefaultStreamingRenderer;
use closeclaw_llm::types::ContentBlock;
use std::sync::Mutex;
use tracing::warn;

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
// ANSI helpers
// ---------------------------------------------------------------------------

/// Strip all ANSI escape sequences from `text`.
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
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

/// Resolve terminal width from an optional size pair.
///
/// When `terminal_size()` returns `Some`, we use the width;
/// otherwise we fall back to the documented default of 80 columns.
pub(crate) fn resolve_terminal_width_from(size: Option<(u16, u16)>) -> usize {
    size.map(|(w, _)| w as usize).unwrap_or(80)
}

/// Return the terminal width in columns, falling back to 80.
pub(crate) fn get_terminal_width() -> usize {
    let size = terminal_size::terminal_size().map(|(w, h)| (w.0, h.0));
    resolve_terminal_width_from(size)
}

// ---------------------------------------------------------------------------
// Markdown → ANSI inline conversion
// ---------------------------------------------------------------------------

/// Parsed inline markdown spans.
pub(crate) enum InlineSpan {
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, url: String },
    Text(String),
}

/// Format a single line of markdown text.
pub(crate) fn format_line(line: &str, ansi: bool) -> String {
    if let Some(styled) = check_line_pattern(line, ansi) {
        return styled;
    }
    let chars: Vec<char> = line.chars().collect();
    let spans = parse_inline_spans(&chars);
    apply_inline_styling(&spans, ansi)
}

/// Check line-level patterns (heading, blockquote, hr).
///
/// Heading detection covers h1 through h6: 1-6 `#` characters followed by
/// a space. 7 or more `#` characters are not a valid markdown heading and
/// are passed through unchanged.
pub(crate) fn check_line_pattern(line: &str, ansi: bool) -> Option<String> {
    // Match h1-h6: 1-6 '#' characters followed by a space
    let bytes = line.as_bytes();
    let mut hash_count = 0;
    while hash_count < 6 && hash_count < bytes.len() && bytes[hash_count] == b'#' {
        hash_count += 1;
    }
    if hash_count > 0 && hash_count < 7 && hash_count < bytes.len() && bytes[hash_count] == b' ' {
        let rest = &line[hash_count + 1..];
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
pub(crate) fn apply_inline_styled_text(text: &str, ansi_code: &str, ansi: bool) -> String {
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
pub(crate) fn apply_inline_styling(spans: &[InlineSpan], ansi: bool) -> String {
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

/// Push accumulated plain text into spans if non-empty.
pub(crate) fn flush_text(spans: &mut Vec<InlineSpan>, chars: &[char], start: usize, end: usize) {
    if start < end {
        let text: String = chars[start..end].iter().collect();
        spans.push(InlineSpan::Text(text));
    }
}

/// Parse inline markdown patterns into spans.
pub(crate) fn parse_inline_spans(chars: &[char]) -> Vec<InlineSpan> {
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

/// Extract bold spans from chars starting at position.
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

// ---------------------------------------------------------------------------
// Code block rendering
// ---------------------------------------------------------------------------

/// ANSI color codes for syntax highlighting.
const ANSI_KEYWORD: &str = "\x1b[35m";
const ANSI_COMMENT: &str = "\x1b[90m";

/// Check if the language has syntax highlighting support.
fn is_supported_language(language: &str) -> bool {
    matches!(
        language,
        "rust" | "python" | "javascript" | "typescript" | "go" | "c" | "cpp" | "java"
    )
}

/// Render a fenced code block with language annotation, line numbers,
/// and optional syntax highlighting.
fn render_code_block_ansi(language: &str, code: &str, ansi: bool) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let line_width = format!("{}", lines.len()).len();
    let mut out = String::new();
    let unsupported = !language.is_empty() && !is_supported_language(language);

    if unsupported {
        out.push_str("```\n");
    }

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

    if unsupported {
        out.push_str("```\n");
    }

    out
}

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
fn render_markdown_ansi(content: &str, ansi: bool) -> String {
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
                out.push_str(&render_code_block_ansi(language, code, ansi));
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

/// Terminal channel outbound renderer.
///
/// Converts [`ContentBlock`]s and DSL parse results into ANSI-formatted text
/// for stdout. Supports markdown formatting, code syntax highlighting,
/// thinking blocks, tool use/result, and DSL element hints.
pub struct TerminalRenderer {
    ansi: bool,
    renderer: Mutex<DefaultStreamingRenderer>,
}

impl TerminalRenderer {
    /// Create a new renderer with auto-detected ANSI capability.
    pub(crate) fn new() -> Self {
        Self {
            ansi: closeclaw_platform::terminal::supports_ansi(),
            renderer: Mutex::new(DefaultStreamingRenderer::new()),
        }
    }

    /// Create a renderer with explicit ANSI mode.
    pub(crate) fn with_ansi(ansi: bool) -> Self {
        Self {
            ansi,
            renderer: Mutex::new(DefaultStreamingRenderer::new()),
        }
    }

    // -- helper methods for non-text content blocks -------------------------

    /// Render a placeholder for unsupported content types.
    fn render_placeholder(&self, kind: &str, name: &str) -> String {
        if self.ansi {
            format!("{}[{}: {}]{}\n", DIM, kind, name, RESET)
        } else {
            format!("[{}: {}]\n", kind, name)
        }
    }

    /// Truncate `content` to terminal width and append `... (truncated)` if needed.
    fn truncate_to_width(&self, content: &str) -> String {
        let max_width = get_terminal_width();
        if content.chars().count() > max_width {
            let truncated: String = content.chars().take(max_width).collect();
            format!("{}... (truncated)", truncated)
        } else {
            content.to_string()
        }
    }

    /// Render a Thinking block with boundary markers.
    fn render_thinking(&self, text: &str) -> String {
        let truncated = self.truncate_to_width(text);
        if self.ansi {
            format!(
                "{}[Thinking]{}\n  {}{}{}[end of thinking]{}\n",
                DIM, RESET, DIM, truncated, DIM, RESET
            )
        } else {
            format!("[Thinking]\n  {}\n[end of thinking]\n", truncated)
        }
    }

    /// Render a ToolUse block: `⚙ tool_name(args)`.
    fn render_tool_use(&self, name: &str, input: &str) -> String {
        let truncated = self.truncate_to_width(input);
        if self.ansi {
            format!(
                "{}⚙ {}{}{}{}({}{}){}{}\n",
                DIM, BOLD, CYAN, name, RESET, DIM, truncated, DIM, RESET
            )
        } else {
            format!("⚙ {}({})\n", name, truncated)
        }
    }

    /// Render a ToolResult block, truncating at terminal width.
    fn render_tool_result(&self, content: &str) -> String {
        let display = self.truncate_to_width(content);
        if self.ansi {
            format!("{}{}{}\n", DIM, display, RESET)
        } else {
            format!("{}\n", display)
        }
    }

    /// Render DSL instructions as plain-text hint lines.
    ///
    /// Terminals cannot render interactive elements, so each instruction
    /// is formatted as a human-readable hint line (e.g. `[Button: ...]`)
    /// and appended to the output. A warning is still logged for diagnostics.
    fn render_dsl(&self, dsl_result: &DslParseResult) -> String {
        let mut lines = Vec::new();
        for inst in &dsl_result.instructions {
            match inst {
                closeclaw_common::processor::DslInstruction::Button {
                    label,
                    action,
                    value,
                } => {
                    warn!(
                        label = %label,
                        action = %action,
                        value = %value,
                        "terminal does not support interactive DSL (Button), skipped"
                    );
                    lines.push(format!("[Button: {} (action: {})]", label, action));
                }
                closeclaw_common::processor::DslInstruction::Selector {
                    label,
                    options,
                    action,
                } => {
                    warn!(
                        label = %label,
                        options = ?options,
                        action = %action,
                        "terminal does not support interactive DSL (Selector), skipped"
                    );
                    let opts = options.join(", ");
                    lines.push(format!(
                        "[Selector: {} (options: {}) (action: {})]",
                        label, opts, action
                    ));
                }
            }
        }
        if lines.is_empty() {
            return String::new();
        }
        let joined = lines.join("\n");
        if self.ansi {
            format!("{}{}{}\n", DIM, joined, RESET)
        } else {
            format!("{}\n", joined)
        }
    }

    /// Wrap `text` with DIM/RESET when ANSI is enabled, otherwise plain.
    #[allow(dead_code)]
    fn dim_or_plain(&self, text: &str) -> String {
        if self.ansi {
            format!("{}{}{}", DIM, text, RESET)
        } else {
            text.to_string()
        }
    }

    /// Render a non-text content block to a string.
    pub(crate) fn render_block(&self, block: &ContentBlock) -> String {
        match block {
            ContentBlock::Text(text) => {
                self.truncate_to_width(&render_markdown_ansi(text, self.ansi))
            }
            ContentBlock::Thinking { thinking: text, .. } => self.render_thinking(text),
            ContentBlock::ToolUse { name, input, .. } => self.render_tool_use(name, input),
            ContentBlock::ToolResult { content, .. } => self.render_tool_result(content),
            ContentBlock::Image(name) => self.render_placeholder("image", name),
            ContentBlock::Audio(name) => self.render_placeholder("audio", name),
            ContentBlock::File(name) => self.render_placeholder("file", name),
        }
    }

    /// Render content blocks and DSL results into a `RenderedOutput`.
    pub(crate) fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let mut output_text = String::new();

        // DSL preprocessing before ContentBlock traversal (design doc requirement)
        if let Some(dsl) = dsl_result {
            let dsl_text = self.render_dsl(dsl);
            if !dsl_text.is_empty() {
                output_text.push_str(&dsl_text);
            }
        }

        for block in content_blocks {
            let rendered = self.render_block(block);
            output_text.push_str(&rendered);
            output_text.push('\n');
        }

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

    /// Render a code block with ANSI line numbers and optional syntax highlighting.
    pub(crate) fn render_code_block(&self, language: &str, code: &str) -> String {
        render_code_block_ansi(language, code, self.ansi)
    }

    /// Render markdown text with ANSI styling.
    pub(crate) fn render_markdown(&self, text: &str) -> String {
        let mut out = String::new();
        for line in text.lines() {
            out.push_str(&format_line(line, self.ansi));
            out.push('\n');
        }
        out
    }

    /// Render a horizontal rule.
    pub(crate) fn render_hr(&self) -> String {
        if self.ansi {
            format!("{}───{}", DIM, RESET)
        } else {
            "───".to_string()
        }
    }

    /// Access the underlying streaming renderer.
    pub(crate) fn streaming_renderer(&self) -> &Mutex<DefaultStreamingRenderer> {
        &self.renderer
    }
}
