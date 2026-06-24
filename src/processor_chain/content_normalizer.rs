//! ContentNormalizer — inbound processor that strips control characters
//! and normalizes Markdown formatting.
//!
//! Processing order:
//! 1. `strip_control_chars` — remove ANSI escape sequences and invisible
//!    control characters (preserving `\n`, `\t`, `\r`)
//! 2. `normalize_empty_lines` — compress consecutive blank lines
//! 3. `trim_trailing_whitespace` — strip trailing spaces per line
//! 4. `normalize_urls` — ensure all bare URLs have `https://` prefix
//! 5. `add_code_block_language_hint` — annotate code fences without language
//!
//! Note: Platform-specific format conversion (e.g. rich-text → Markdown)
//! is handled by each IM plugin during parsing, not by this processor.

use crate::processor_chain::context::{MessageContext, ProcessedMessage};
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use async_trait::async_trait;
use std::sync::LazyLock;

use regex::Regex;

// ---------------------------------------------------------------------------
// Platform residue stripping
// ---------------------------------------------------------------------------

/// Strips platform-specific XML-style tags from message text.
///
/// Converts `<at user_id="xxx">name</at>` to `@name`, removing the
/// platform-specific at-mention markup while preserving the display name.
/// Handles multiple and nested tags; non-matching text passes through
/// unchanged.
pub fn strip_platform_residue(text: &str) -> String {
    static AT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<at\s+user_id="[^"]*">([^<]*)</at>"#).unwrap());
    AT_RE.replace_all(text, "@$1").to_string()
}

// ---------------------------------------------------------------------------
// Control character stripping
// ---------------------------------------------------------------------------

/// Removes ANSI escape sequences and invisible control characters from text.
///
/// Preserves `\n`, `\t`, and `\r`. Strips everything else in the 0x00–0x1F
/// range except those three, as well as ANSI CSI sequences (`\x1b[...`).
pub fn strip_control_chars(text: &str) -> String {
    // Strip ANSI escape sequences: \x1b followed by any CSI parameters/commands.
    static ANSI_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap());
    let stripped = ANSI_RE.replace_all(text, "");

    // Strip remaining control characters except \n (0x0A), \t (0x09), \r (0x0D).
    stripped
        .chars()
        .filter(|&c| {
            let is_control = c as u32 <= 0x1F;
            !is_control || c == '\n' || c == '\t' || c == '\r'
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Normalization functions
// ---------------------------------------------------------------------------

/// Compresses two or more consecutive empty lines into a single empty line.
pub fn normalize_empty_lines(text: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{2,}").unwrap());
    RE.replace_all(text, "\n\n").to_string()
}

/// Removes trailing whitespace from every line.
pub fn trim_trailing_whitespace(text: &str) -> String {
    text.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Adds `https://` prefix to bare URLs that lack an http/https scheme.
///
/// Skips URLs already inside markdown link syntax `[text](url)`.
pub fn normalize_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let len = text.len();
    let mut i = 0;

    while i < len {
        // Skip non-ASCII bytes (multi-byte UTF-8) — just copy them as a string slice
        if !bytes[i].is_ascii() {
            let start = i;
            i += 1;
            while i < len && !bytes[i].is_ascii() {
                i += 1;
            }
            out.push_str(&text[start..i]);
            continue;
        }

        // Skip markdown links [text](url)
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < len && bytes[j] != b']' {
                j += 1;
            }
            if j < len && j + 1 < len && bytes[j + 1] == b'(' {
                let mut k = j + 1;
                while k < len && bytes[k] != b')' {
                    k += 1;
                }
                out.push_str(&text[i..=k]);
                i = k + 1;
                continue;
            }
            out.push('[');
            i += 1;
            continue;
        }

        if i + 4 <= len && &text[i..i + 4] == "www." {
            out.push_str("https://www.");
            i += 4;
            while i < len
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b'"'
                && bytes[i] != b'\''
                && bytes[i] != b')'
                && bytes[i] != b']'
            {
                out.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }

        let preceded_by_scheme =
            i >= 3 && bytes[i - 3] == b':' && bytes[i - 2] == b'/' && bytes[i - 1] == b'/';
        if !preceded_by_scheme
            && i > 0
            && !bytes[i - 1].is_ascii_alphanumeric()
            && i < len
            && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'.')
        {
            let start = i;
            let mut j = i;
            while j < len
                && !bytes[j].is_ascii_whitespace()
                && bytes[j] != b'"'
                && bytes[j] != b'\''
                && bytes[j] != b'<'
                && bytes[j] != b')'
                && bytes[j] != b']'
            {
                j += 1;
            }
            let token = &text[start..j];

            if token.contains('.')
                && !token.starts_with("http://")
                && !token.starts_with("https://")
                && !token.starts_with("ftp://")
                && !token.starts_with("file://")
            {
                out.push_str("https://");
                out.push_str(token);
                i = j;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

/// Adds ` ```text` language hint to code blocks that lack a language tag.
pub fn add_code_block_language_hint(text: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(```)([^\w\n]|$)").unwrap());
    RE.replace_all(text, "```text$1").to_string()
}

// ---------------------------------------------------------------------------
// ContentNormalizer
// ---------------------------------------------------------------------------

/// ContentNormalizer strips control characters and normalizes Markdown
/// formatting in message content.
///
/// Processing order:
/// 1. `strip_control_chars` — remove ANSI sequences and invisible characters
/// 2. `normalize_empty_lines` — compress consecutive blank lines
/// 3. `trim_trailing_whitespace` — strip trailing spaces per line
/// 4. `normalize_urls` — ensure bare URLs have `https://` prefix
/// 5. `add_code_block_language_hint` — annotate unlabeled code fences
///
/// Note: Platform-specific format conversion is handled by each IM plugin
/// during parsing, not by this processor.
#[derive(Debug)]
pub struct ContentNormalizer;

impl ContentNormalizer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ContentNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MessageProcessor for ContentNormalizer {
    fn name(&self) -> &str {
        "content_normalizer"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        30
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let mut normalized = strip_control_chars(&ctx.content);
        normalized = normalize_empty_lines(&normalized);
        normalized = trim_trailing_whitespace(&normalized);
        normalized = normalize_urls(&normalized);
        normalized = add_code_block_language_hint(&normalized);

        Ok(Some(ProcessedMessage {
            content: normalized,
            metadata: serde_json::Map::new(),
            suppress: false,
            content_blocks: vec![],
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "content_normalizer_tests.rs"]
mod tests;
