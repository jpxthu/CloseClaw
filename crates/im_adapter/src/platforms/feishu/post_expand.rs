//! Post content expansion — converts Feishu post JSON to plain text.

use super::text_style::apply_text_style;

// Post content expansion
/// Expand a Feishu post-type content JSON value into plain text.
///
/// The `content` parameter is the parsed JSON object with `title` (optional)
/// and `content` (2D array of elements, each element has a `tag` field).
///
/// - `title` becomes the first line (if present).
/// - Each sub-array in `content` becomes one line; elements are concatenated.
/// - Supported tags: `text`, `a`, `at`, unknown tags use `text` if available.
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
/// - unknown tags → text if available, otherwise `[未知消息]`
pub(crate) fn expand_element(elem: &serde_json::Value) -> String {
    let tag = elem.get("tag").and_then(|t| t.as_str()).unwrap_or("");
    match tag {
        "text" | "a" => elem
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "at" => {
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
        "text_run" => {
            let text = elem.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let style = elem
                .get("style")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            apply_text_style(text, &style)
        }
        "img" => "[图片]".to_string(),
        "media" => "[视频]".to_string(),
        "file" => "[文件]".to_string(),
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
