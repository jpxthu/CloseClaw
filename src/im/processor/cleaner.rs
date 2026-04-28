//! FeishuMessageCleaner — inbound MessageProcessor for feishu webhook events.

use async_trait::async_trait;
use std::collections::BTreeMap;

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase, ProcessedMessage};
use serde_json::Value;

// ---------------------------------------------------------------------------
// FeishuMessageCleaner
// ---------------------------------------------------------------------------

/// Cleans raw feishu webhook events, producing plain-text messages.
///
/// Implements [`MessageProcessor`] with [`ProcessPhase::Inbound`] and priority 30.
#[derive(Debug, Clone, Default)]
pub struct FeishuMessageCleaner;

#[async_trait]
impl MessageProcessor for FeishuMessageCleaner {
    fn priority(&self) -> i32 {
        30
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    async fn process(
        &self,
        _ctx: &MessageContext,
        raw: &Value,
    ) -> Result<ProcessedMessage, ProcessError> {
        clean_message(raw)
    }
}

// ---------------------------------------------------------------------------
// Internal cleaning logic (migrated from processor.rs)
// ---------------------------------------------------------------------------

fn clean_message(raw: &Value) -> Result<ProcessedMessage, ProcessError> {
    let msg = raw.get("message").ok_or(ProcessError::MissingMessage)?;

    let msg_type = msg
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let content = match msg_type {
        "text" => clean_text_message(msg),
        "post" => clean_post_message(msg),
        _ => msg
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    };

    let mut metadata = BTreeMap::new();
    if let Some(chat_type) = msg.get("chat_type").and_then(|v| v.as_str()) {
        if chat_type == "group" {
            metadata.insert("chat_type".to_string(), chat_type.to_string());
        }
    }

    Ok(ProcessedMessage { content, metadata })
}

// ---------------------------------------------------------------------------
// Text message cleaner
// ---------------------------------------------------------------------------

fn clean_text_message(msg: &Value) -> String {
    let content_str = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

    let parsed: Value = match serde_json::from_str(content_str) {
        Ok(v) => v,
        Err(_) => return content_str.to_string(),
    };

    let text = parsed.get("text").and_then(|v| v.as_str()).unwrap_or("");

    let mentions = msg.get("mentions").and_then(|v| v.as_array());
    if let Some(mentions) = mentions {
        replace_mentions(text, mentions)
    } else {
        text.to_string()
    }
}

// ---------------------------------------------------------------------------
// Post message cleaner
// ---------------------------------------------------------------------------

fn clean_post_message(msg: &Value) -> String {
    let content_str = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

    let parsed: Value = match serde_json::from_str(content_str) {
        Ok(v) => v,
        Err(_) => return content_str.to_string(),
    };

    let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let content_blocks = parsed.get("content").and_then(|v| v.as_array());

    let mut lines: Vec<String> = Vec::new();

    if !title.is_empty() {
        lines.push(title.to_string());
    }

    if let Some(blocks) = content_blocks {
        let rendered_blocks = render_blocks(blocks);
        lines.extend(rendered_blocks);
    }

    lines.join("\n")
}

fn render_blocks(blocks: &[Value]) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for b in blocks {
        let arr = match b.as_array() {
            Some(a) => a,
            None => continue,
        };

        if arr.is_empty() {
            result.push(String::new());
        } else {
            let rendered = render_post_block(arr);
            if rendered == "[图片]" {
                result.push(String::new());
                result.push(rendered);
            } else {
                let is_heading = rendered.starts_with('#');
                result.push(rendered);
                if is_heading {
                    result.push(String::new());
                }
            }
        }
    }
    result
}

fn render_post_block(segments: &[Value]) -> String {
    segments
        .iter()
        .map(render_segment)
        .collect::<Vec<_>>()
        .join("")
}

fn render_segment(seg: &Value) -> String {
    let tag = seg.get("tag").and_then(|v| v.as_str()).unwrap_or("");

    match tag {
        "text" => {
            let text = seg.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let styles = seg
                .get("style")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str())
                        .map(String::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let styled = apply_styles(text, &styles);
            if text == "引用" {
                format!("> {}", styled)
            } else {
                styled
            }
        }
        "img" => "[图片]".to_string(),
        _ => String::new(),
    }
}

fn apply_styles(text: &str, styles: &[String]) -> String {
    if styles.is_empty() {
        return text.to_string();
    }

    let styles_str: Vec<&str> = styles.iter().map(|s| s.as_str()).collect();

    if styles_str == ["underline", "bold"] {
        return format!("**<u>{}</u>**", text);
    } else if styles_str == ["lineThrough", "underline"] {
        return format!("~~<u>{}</u>~~", text);
    } else if styles_str == ["lineThrough", "underline", "bold"] {
        return format!("**<u>~~{}~~</u>**", text);
    }

    let mut result = text.to_string();
    for s in styles.iter().rev() {
        result = wrap_style(&result, s);
    }
    result
}

fn wrap_style(text: &str, style: &str) -> String {
    let (open, close) = match style {
        "bold" => ("**", "**"),
        "italic" => ("_", "_"),
        "underline" => ("<u>", "</u>"),
        "lineThrough" => ("~~", "~~"),
        _ => return text.to_string(),
    };
    format!("{}{}{}", open, text, close)
}

// ---------------------------------------------------------------------------
// Mention replacement
// ---------------------------------------------------------------------------

fn replace_mentions(text: &str, mentions: &[Value]) -> String {
    let mut result = text.to_string();
    for mention in mentions {
        let key = mention.get("key").and_then(|v| v.as_str()).unwrap_or("");
        let name = mention.get("name").and_then(|v| v.as_str()).unwrap_or(key);
        result = result.replace(key, &format!("@{}", name));
    }
    result
}
