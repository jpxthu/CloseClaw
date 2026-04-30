//! FeishuMessageCleaner — inbound MessageProcessor for feishu webhook events.

use async_trait::async_trait;
use std::collections::BTreeMap;

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase, ProcessedMessage};
use serde_json::Value;

/// Metadata key for the original feishu `message` object (preserved by
/// FeishuMessageCleaner so downstream processors can still access raw fields).
const META_ORIGINAL_MESSAGE: &str = "_orig_message";

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
        ctx: &MessageContext,
        raw: &Value,
    ) -> Result<ProcessedMessage, ProcessError> {
        clean_message(raw, ctx)
    }
}

// ---------------------------------------------------------------------------
// Internal cleaning logic (migrated from processor.rs)
// ---------------------------------------------------------------------------

fn clean_message(raw: &Value, ctx: &MessageContext) -> Result<ProcessedMessage, ProcessError> {
    // Detect whether raw is a raw feishu webhook (has "message" field) or a
    // ProcessedMessage from an earlier processor (has "content" field).
    let (content, orig_msg_opt) = if let Some(msg) = raw.get("message") {
        // Raw feishu webhook.
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
        (content, Some(msg.clone()))
    } else {
        // Already a ProcessedMessage — content is a JSON string from SessionRouter
        // that encodes the feishu inner message content (e.g. {"text":"..."} or
        // {"title":"...","content":[[...]]}). Detect message type from the content keys.
        let content_str = raw.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let content = if let Ok(parsed) = serde_json::from_str::<Value>(content_str) {
            // Detect type from content keys: "text" → text msg, "title" → post msg.
            if parsed.get("text").is_some() {
                // Text message: extract the "text" field value.
                parsed
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or(content_str)
                    .to_string()
            } else if parsed.get("title").is_some() || parsed.get("content").is_some() {
                // Post message: render title + content blocks.
                let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let blocks = parsed
                    .get("content")
                    .and_then(|v| v.as_array())
                    .map(|v| render_blocks(v.as_slice()))
                    .unwrap_or_default();
                let mut lines = Vec::new();
                if !title.is_empty() {
                    lines.push(title.to_string());
                }
                lines.extend(blocks);
                lines.join("\n")
            } else {
                // Unknown format: return as-is.
                content_str.to_string()
            }
        } else {
            content_str.to_string()
        };
        (content, None)
    };

    // Preserve upstream metadata (e.g. session_id from SessionRouter) and
    // only add/override with what this cleaner produces.
    let mut metadata = ctx.metadata.clone();

    // If we saw a raw feishu message, preserve it for downstream processors.
    if let Some(msg_val) = orig_msg_opt {
        metadata.insert(META_ORIGINAL_MESSAGE.to_string(), msg_val.to_string());
        if let Some(chat_type) = msg_val.get("chat_type").and_then(|v| v.as_str()) {
            if chat_type == "group" {
                metadata.insert("chat_type".to_string(), chat_type.to_string());
            } else {
                metadata.remove("chat_type");
            }
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
