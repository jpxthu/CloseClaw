//! ContentNormalizer — inbound processor that strips control characters
//! and normalizes Markdown formatting.
//!
//! Processing order:
//! 1. `strip_control_chars` — remove ANSI escape sequences and invisible
//!    control characters (preserving `\n`, `\t`, `\r`)
//! 2. `normalize_empty_lines` — compress consecutive blank lines
//! 3. `trim_trailing_whitespace` — strip trailing spaces per line
//!
//! Note: Platform-specific format conversion (e.g. rich-text → Markdown),
//! URL completion, and code block language hints are handled by each IM
//! plugin during parsing, not by this processor.

use crate::processor_chain::context::{MessageContext, ProcessedMessage};
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use async_trait::async_trait;
use closeclaw_llm::types::ContentBlock;
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
///
/// Note: Platform-specific format conversion, URL completion, and code block
/// language hints are handled by each IM plugin during parsing, not by this
/// processor.
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

        Ok(Some(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(normalized)],
            metadata: std::collections::HashMap::new(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "content_normalizer_tests.rs"]
mod tests;
