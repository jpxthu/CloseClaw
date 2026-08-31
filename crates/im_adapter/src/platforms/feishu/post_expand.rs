//! Post content expansion — converts Feishu post JSON to plain text.

use super::text_style::apply_text_style;
use closeclaw_common::{MediaRef, MediaType};

// Post content expansion
/// Expand a Feishu post-type content JSON value into plain text.
///
/// The `content` parameter is the parsed JSON object with `title` (optional)
/// and `content` (2D array of elements, each element has a `tag` field).
///
/// - `title` becomes the first line (if present).
/// - Each sub-array in `content` becomes one line; elements are concatenated.
/// - Supported tags: `text`, `a`, `at`, `code_block`, `code`/`inline_code`, unknown tags use `text` if available.
pub(crate) fn expand_post_content(content: &serde_json::Value) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Extract title as the first line if present.
    if let Some(title) = content.get("title").and_then(|t| t.as_str()) {
        if !title.is_empty() {
            lines.push(title.to_string());
        }
    }

    // Iterate over the 2D content array.
    if let Some(rows) = content.get("content").and_then(|c| c.as_array()) {
        for row in rows {
            let row_text: String = row
                .as_array()
                .map(|elements| {
                    elements
                        .iter()
                        .map(expand_element)
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            lines.push(row_text);
        }
    }
    lines.join("\n")
}

/// Expand a single post content element into plain text based on its tag.
///
/// Supported tags:
/// - `text`, `a` → text content
/// - `at` → `@name` or `@user_id`
/// - `text_run` → styled text via `apply_text_style`
/// - `img` → `[图片]`
/// - `media` → `[视频]`
/// - `file` → `[文件]`
/// - `emoji` → `[emoji_type]` placeholder
/// - `code_block`, `inline_code` → fenced / inline code
/// - `quote` → recursively expanded, rendered as markdown blockquote (`> ` prefix per line)
/// - unknown tags → text if available, otherwise `[未知消息]`
pub(crate) fn expand_element(elem: &serde_json::Value) -> String {
    let tag = elem.get("tag").and_then(|t| t.as_str()).unwrap_or("");
    match tag {
        "text" | "a" => elem
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "at" => expand_at(elem),
        "text_run" => expand_text_run(elem),
        "img" => "[图片]".to_string(),
        "emoji" => expand_emoji(elem),
        "media" => "[视频]".to_string(),
        "file" => "[文件]".to_string(),
        "code_block" => expand_code_block(elem),
        "code" | "inline_code" => {
            let text = elem.get("text").and_then(|t| t.as_str()).unwrap_or("");
            format!("`{}`", text)
        }
        "quote" => expand_quote(elem),
        _ => {
            let text = elem.get("text").and_then(|t| t.as_str()).unwrap_or("");
            if text.is_empty() {
                "[未知消息]".to_string()
            } else {
                text.to_string()
            }
        }
    }
}

// --- Helper functions for expand_element branches ---

fn expand_at(elem: &serde_json::Value) -> String {
    if let Some(name) = elem.get("name").and_then(|n| n.as_str()) {
        format!("@{}", name)
    } else if let Some(user_id) = elem.get("user_id").and_then(|u| u.as_str()) {
        format!("@{}", user_id)
    } else {
        elem.get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string()
    }
}

fn expand_text_run(elem: &serde_json::Value) -> String {
    let text = elem.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let style = elem
        .get("style")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    apply_text_style(text, &style)
}

fn expand_emoji(elem: &serde_json::Value) -> String {
    let emoji_type = elem
        .get("emoji_type")
        .and_then(|e| e.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    if emoji_type.is_empty() {
        "[未知消息]".to_string()
    } else {
        format!("[{}]", emoji_type)
    }
}

fn expand_code_block(elem: &serde_json::Value) -> String {
    let text = elem.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let lang = elem
        .get("language")
        .and_then(|l| l.as_str())
        .filter(|s| !s.is_empty());
    let opening = match lang {
        Some(l) => format!("```{}", l),
        None => "```".to_string(),
    };
    if text.is_empty() {
        format!("{}\n```", opening)
    } else {
        format!("{}\n{}\n```", opening, text)
    }
}

fn expand_quote(elem: &serde_json::Value) -> String {
    let inner = if let Some(elements) = elem.get("elements") {
        // Flat element array: expand each and join.
        elements
            .as_array()
            .map(|arr| arr.iter().map(expand_element).collect::<Vec<_>>().join(""))
            .unwrap_or_default()
    } else if let Some(content) = elem.get("content") {
        // 2D content array: reuse expand_post_content for nested rows.
        let wrapper = serde_json::json!({"content": content});
        expand_post_content(&wrapper)
    } else {
        String::new()
    };
    if inner.is_empty() {
        String::new()
    } else {
        inner
            .lines()
            .map(|line| format!("> {}", line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Extract media references from a single post content element.
/// Returns `Some(MediaRef)` for `img`, `media`, and `file` tags with
/// a non-empty key; `None` for all other tags or missing/empty keys.
fn media_ref_from_elem(elem: &serde_json::Value) -> Option<MediaRef> {
    let tag = elem.get("tag").and_then(|t| t.as_str())?;
    let (key, media_type) = match tag {
        "img" => (elem.get("image_key")?, MediaType::Image),
        "media" | "file" => (
            // Design decision: `media` tag maps to
            // MediaType::File (not a distinct Video variant)
            // because the current adapter metadata has no
            // finer-grained media sub-type for video vs file.
            // This is consistent with the existing file|audio
            // handling in the adapter.
            elem.get("file_key")?,
            MediaType::File,
        ),
        _ => return None,
    };
    let key = key.as_str()?.to_string();
    if key.is_empty() {
        return None;
    }
    Some(MediaRef {
        key,
        path: String::new(),
        media_type,
        size: 0,
        mime: String::new(),
    })
}

/// Extract media references from a post message's 2D content array.
/// Scans for `img`, `media`, and `file` tags and builds `MediaRef`
/// entries for each embedded resource.
pub(crate) fn extract_post_media_refs(content: &serde_json::Value) -> Vec<MediaRef> {
    let Some(rows) = content.get("content").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .flat_map(|row| row.as_array().into_iter().flatten())
        .filter_map(media_ref_from_elem)
        .collect()
}
