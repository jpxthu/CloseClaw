//! ContentNormalizer — inbound processor that merges feishu message
//! extraction and markdown normalization into a single step.

use crate::processor_chain::context::{MessageContext, ProcessedMessage};
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::LazyLock;

use regex::Regex;

// ---------------------------------------------------------------------------
// Feishu webhook JSON structures
// ---------------------------------------------------------------------------

/// Feishu webhook text message content.
#[derive(Debug, Deserialize)]
struct TextContent {
    text: String,
}

/// Feishu webhook message envelope.
#[derive(Debug, Deserialize)]
struct FeishuMessage {
    #[serde(rename = "msg_type")]
    msg_type: String,
    #[serde(rename = "text", default)]
    text: Option<TextContent>,
    #[serde(rename = "content", default)]
    content: Option<serde_json::Value>,
    #[serde(rename = "thread_id", default)]
    thread_id: Option<String>,
    #[serde(rename = "root_id", default)]
    root_id: Option<String>,
    #[serde(rename = "parent_id", default)]
    parent_id: Option<String>,
}

/// A Feishu post message content field.
#[derive(Debug, Deserialize)]
struct PostContent {
    title: String,
    content: Vec<Vec<PostSegment>>,
}

/// A single segment within a post paragraph.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct PostSegment {
    tag: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    style: Vec<String>,
    #[serde(rename = "user_id", default)]
    user_id: Option<String>,
    #[serde(rename = "user_name", default)]
    user_name: Option<String>,
    #[serde(default)]
    href: Option<String>,
    #[serde(rename = "image_key", default)]
    image_key: Option<String>,
    #[serde(rename = "video_key", default)]
    video_key: Option<String>,
    #[serde(rename = "audio_key", default)]
    audio_key: Option<String>,
    #[serde(rename = "email", default)]
    email: Option<String>,
    #[serde(rename = "phone_number", default)]
    phone_number: Option<String>,
}

// ---------------------------------------------------------------------------
// post_to_markdown
// ---------------------------------------------------------------------------

/// Converts a Feishu post message JSON string to Markdown.
///
/// Handles all Feishu rich-text tags: text, at, link, email, phone,
/// channel_at, media/img, bold, italic, strike/lineThrough, underline, code,
/// blockquote.
pub fn post_to_markdown(post_json: &str) -> String {
    let post: PostContent = match serde_json::from_str(post_json) {
        Ok(p) => p,
        Err(_) => return post_json.to_string(),
    };

    let mut out = String::new();

    if !post.title.is_empty() {
        out.push_str(&format!("## {}\n", post.title));
    }

    for (pi, paragraph) in post.content.iter().enumerate() {
        if paragraph.is_empty() {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            continue;
        }

        let para_text = paragraph_to_markdown(paragraph);
        out.push_str(&para_text);

        if pi + 1 < post.content.len() {
            out.push('\n');
        }
    }

    out.trim_end().to_string()
}

fn paragraph_to_markdown(paragraph: &[PostSegment]) -> String {
    paragraph
        .iter()
        .map(segment_to_markdown)
        .collect::<String>()
}

fn segment_to_markdown(seg: &PostSegment) -> String {
    let raw = match seg.tag.as_str() {
        "text" => seg.text.clone(),
        "at" => {
            let name = seg.user_name.as_deref().unwrap_or("");
            if name.is_empty() {
                "@某人".to_string()
            } else {
                format!("@{}", name)
            }
        }
        "link" => {
            let text = &seg.text;
            let href = seg.href.as_deref().unwrap_or("");
            if text.is_empty() {
                format!("[链接]({})", href)
            } else {
                format!("[{}]({})", text, href)
            }
        }
        "email" => {
            let addr = seg.email.as_deref().unwrap_or("");
            format!("<mailto:{}>", addr)
        }
        "phone" => {
            let num = seg.phone_number.as_deref().unwrap_or("");
            format!("<tel:{}>", num)
        }
        "channel_at" => {
            let name = seg.text.as_str();
            if name.is_empty() {
                "@频道".to_string()
            } else {
                format!("@{}", name)
            }
        }
        "img" | "media" => "[图片]".to_string(),
        "video" => "[视频]".to_string(),
        "audio" => "[音频]".to_string(),
        "button" => seg.text.clone(),
        _ => seg.text.clone(),
    };

    apply_inline_styles(&raw, &seg.style)
}

fn apply_inline_styles(text: &str, styles: &[String]) -> String {
    if text.is_empty() || styles.is_empty() {
        return text.to_string();
    }

    let mut result = text.to_string();

    let has_bold = styles.contains(&"bold".to_string());
    let has_underline = styles.contains(&"underline".to_string());
    let has_line_through =
        styles.contains(&"lineThrough".to_string()) || styles.contains(&"strike".to_string());
    let has_italic = styles.contains(&"italic".to_string());
    let has_code = styles.contains(&"code".to_string());

    if has_code {
        result = format!("`{}`", result);
    }

    if has_line_through && has_underline && has_bold {
        result = format!("~~{}~~", result);
        result = format!("<u>{}</u>", result);
        result = format!("**{}**", result);
    } else if has_line_through && has_underline && !has_bold {
        result = format!("<u>{}</u>", result);
        result = format!("~~{}~~", result);
    } else if has_underline && has_bold {
        result = format!("<u>{}</u>", result);
        result = format!("**{}**", result);
    } else {
        if has_line_through {
            result = format!("~~{}~~", result);
        }
        if has_underline {
            result = format!("<u>{}</u>", result);
        }
        if has_italic {
            result = format!("*{}*", result);
        }
        if has_bold {
            result = format!("**{}**", result);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Normalization functions (from MarkdownNormalizer)
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

/// ContentNormalizer combines feishu message extraction and markdown
/// normalization into a single inbound processor.
///
/// Processing order:
/// 1. Parse feishu webhook JSON → extract text content
/// 2. `normalize_empty_lines` — compress consecutive blank lines
/// 3. `trim_trailing_whitespace` — strip trailing spaces per line
/// 4. `normalize_urls` — ensure all bare URLs have https:// prefix
/// 5. `add_code_block_language_hint` — annotate code fences without language
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
        // Step 1: Parse feishu webhook content.
        let raw: FeishuMessage = serde_json::from_str(&ctx.content).map_err(|e| {
            ProcessError::invalid_message(format!("failed to parse feishu message: {e}"))
        })?;

        let content = match raw.msg_type.as_str() {
            "text" => {
                let text = raw.text.as_ref().ok_or_else(|| {
                    ProcessError::invalid_message("text message missing text field")
                })?;
                text.text.clone()
            }
            "post" => {
                let content_json = raw.content.as_ref().ok_or_else(|| {
                    ProcessError::invalid_message("post message missing content field")
                })?;
                let content_str = content_json.to_string();
                post_to_markdown(&content_str)
            }
            _ => return Ok(None),
        };

        // Extract feishu_thread_id: prefer thread_id > root_id > parent_id
        let feishu_thread_id = raw.thread_id.or(raw.root_id).or(raw.parent_id);

        let mut metadata = serde_json::Map::new();
        if let Some(tid) = feishu_thread_id {
            metadata.insert("feishu_thread_id".to_string(), serde_json::json!(tid));
        }

        // Step 2–5: Markdown normalization
        let mut normalized = content;
        normalized = normalize_empty_lines(&normalized);
        normalized = trim_trailing_whitespace(&normalized);
        normalized = normalize_urls(&normalized);
        normalized = add_code_block_language_hint(&normalized);

        Ok(Some(ProcessedMessage {
            content: normalized,
            metadata,
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
