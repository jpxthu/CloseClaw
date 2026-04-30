//! MarkdownNormalizer — inbound processor that standardizes markdown content
//! produced by MessageCleaner, ensuring consistent formatting before LLM input.

use crate::processor_chain::context::{MessageContext, ProcessedMessage};
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use async_trait::async_trait;
use std::sync::LazyLock;

use regex::Regex;

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
        // Skip markdown links [text](url)
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < len && bytes[j] != b']' {
                j += 1;
            }
            if j < len && j + 1 < len && bytes[j + 1] == b'(' {
                // It's a markdown link — copy everything until matching )
                let mut k = j + 1;
                while k < len && bytes[k] != b')' {
                    k += 1;
                }
                out.push_str(&text[i..=k]);
                i = k + 1;
                continue;
            }
            // Not a markdown link — copy [ and continue
            out.push('[');
            i += 1;
            continue;
        }

        // Handle www. prefix — add https://
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

        // Handle bare domain — has at least one dot, not preceded by word char
        // A bare domain: starts with letter/digit, contains at least one dot,
        // not preceded by :// or already has a scheme.
        // We scan ahead to find a candidate domain-like token.
        // Also skip if preceded by :// (inside an existing scheme URL like http://...)
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

            // Check if it looks like a domain (has a dot, no scheme yet)
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
// MarkdownNormalizer
// ---------------------------------------------------------------------------

/// MarkdownNormalizer standardizes markdown content before LLM input.
///
/// Processing order:
/// 1. `normalize_empty_lines` — compress consecutive blank lines
/// 2. `trim_trailing_whitespace` — strip trailing spaces per line
/// 3. `normalize_urls` — ensure all bare URLs have https:// prefix
/// 4. `add_code_block_language_hint` — annotate code fences without language
#[derive(Debug)]
pub struct MarkdownNormalizer;

impl MarkdownNormalizer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MarkdownNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MessageProcessor for MarkdownNormalizer {
    fn name(&self) -> &str {
        "markdown_normalizer"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        40
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let mut content = ctx.content.clone();

        content = normalize_empty_lines(&content);
        content = trim_trailing_whitespace(&content);
        content = normalize_urls(&content);
        content = add_code_block_language_hint(&content);

        Ok(Some(ProcessedMessage {
            content,
            metadata: ctx.metadata.clone(),
            suppress: false,
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // normalize_empty_lines

    #[test]
    fn test_normalize_empty_lines_three_plus() {
        let input = "hello\n\n\n\nworld";
        let out = normalize_empty_lines(input);
        assert_eq!(out, "hello\n\nworld");
    }

    #[test]
    fn test_normalize_empty_lines_two() {
        let input = "hello\n\nworld";
        let out = normalize_empty_lines(input);
        assert_eq!(out, "hello\n\nworld");
    }

    #[test]
    fn test_normalize_empty_lines_single() {
        let input = "hello\nworld";
        let out = normalize_empty_lines(input);
        assert_eq!(out, "hello\nworld");
    }

    // trim_trailing_whitespace

    #[test]
    fn test_trim_trailing_whitespace_with_space() {
        let input = "hello   \nworld  ";
        let out = trim_trailing_whitespace(input);
        assert_eq!(out, "hello\nworld");
    }

    #[test]
    fn test_trim_trailing_whitespace_without_space() {
        let input = "hello\nworld";
        let out = trim_trailing_whitespace(input);
        assert_eq!(out, "hello\nworld");
    }

    // normalize_urls

    #[test]
    fn test_normalize_urls_www() {
        let input = "then www.example.com also";
        let out = normalize_urls(input);
        assert_eq!(out, "then https://www.example.com also");
    }

    #[test]
    fn test_normalize_urls_bare_domain() {
        let input = "visit google.com/path please";
        let out = normalize_urls(input);
        assert_eq!(out, "visit https://google.com/path please", "got: {out}");
    }

    #[test]
    fn test_normalize_urls_http_unchanged() {
        let input = "see http://example.com ok";
        let out = normalize_urls(input);
        assert_eq!(out, "see http://example.com ok");
    }

    #[test]
    fn test_normalize_urls_https_unchanged() {
        let input = "see https://example.com ok";
        let out = normalize_urls(input);
        assert_eq!(out, "see https://example.com ok");
    }

    #[test]
    fn test_normalize_urls_in_markdown_link_unchanged() {
        let input = "see [example](www.example.com) link";
        let out = normalize_urls(input);
        assert_eq!(out, "see [example](www.example.com) link", "got: {out}");
    }

    // add_code_block_language_hint

    #[test]
    fn test_add_language_hint_unlabeled() {
        let input = "```\ncode here\n```";
        let out = add_code_block_language_hint(input);
        assert!(out.contains("```text"), "got: {out}");
    }

    #[test]
    fn test_add_language_hint_labeled_unchanged() {
        let input = "```rust\nfn main() {}\n```";
        let out = add_code_block_language_hint(input);
        assert!(out.contains("```rust"));
        assert!(!out.contains("```text```rust"), "got: {out}");
    }

    #[test]
    fn test_add_code_block_normal_text_unchanged() {
        let input = "just some plain text";
        let out = add_code_block_language_hint(input);
        assert_eq!(out, "just some plain text");
    }
}
