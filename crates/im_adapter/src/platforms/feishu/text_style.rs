//! Text style helpers for Feishu post content rendering.
//!
//! Converts `text_run` style attributes (bold, italic, strikethrough, underline)
//! into markdown-equivalent formatting. Link styles are intentionally discarded.

/// Check a boolean style flag from the style JSON object.
pub(crate) fn bool_style(style: &serde_json::Value, key: &str) -> bool {
    style.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Wrap text with inline markdown styles based on the style object.
///
/// Supported styles: bold, italic, strikethrough, underline.
/// Combination styles are applied in order: outer styles (strikethrough)
/// wrap inner styles (bold).
pub(crate) fn wrap_inline_styles(text: &str, style: &serde_json::Value) -> String {
    let mut result = text.to_string();
    if bool_style(style, "bold") {
        result = format!("**{}**", result);
    }
    if bool_style(style, "italic") {
        result = format!("_{}_", result);
    }
    if bool_style(style, "strikethrough") {
        result = format!("~~{}~~", result);
    }
    if bool_style(style, "underline") {
        result = format!("<u>{}</u>", result);
    }
    result
}

/// Apply text styles to a text_run element's content.
///
/// Supported styles: bold, italic, strikethrough, underline.
/// Link styles are intentionally discarded per design doc.
pub(crate) fn apply_text_style(text: &str, style: &serde_json::Value) -> String {
    wrap_inline_styles(text, style)
}
