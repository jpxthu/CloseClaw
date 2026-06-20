//! FeishuParser — inbound MessageProcessor that parses feishu webhook JSON
//! and extracts plain text content.
//!
//! Handles `text` and `post` message types, converts rich-text post content
//! to Markdown, and extracts `feishu_thread_id` from thread/root/parent IDs.
//!
//! Runs at priority 25, between SessionRouter (20) and FeishuMessageCleaner (30).

use super::{MessageContext, MessageProcessor, ProcessError, ProcessPhase, ProcessedMessage};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Feishu webhook JSON structures
// ---------------------------------------------------------------------------

/// Feishu webhook `message` object (inside the top-level `message` field).
///
/// The `content` field is a JSON-encoded string whose inner format varies
/// by `message_type`:
/// - `"text"` → `{"text": "..."}`
/// - `"post"` → `{"title": "...", "content": [[...]]}`
#[derive(Debug, Deserialize)]
struct FeishuMessage {
    #[serde(rename = "message_type")]
    msg_type: String,
    /// Inner content as a raw JSON string.
    content: Option<String>,
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
// FeishuParser
// ---------------------------------------------------------------------------

/// Parses feishu webhook JSON into plain-text content.
///
/// Extracts text from `text` messages and converts `post` rich-text to
/// Markdown. Also extracts `feishu_thread_id` metadata.
#[derive(Debug, Clone, Default)]
pub struct FeishuParser;

impl FeishuParser {
    /// Create a new FeishuParser.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MessageProcessor for FeishuParser {
    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    fn priority(&self) -> i32 {
        25 // after SessionRouter (20), before FeishuMessageCleaner (30)
    }

    async fn process(
        &self,
        ctx: &MessageContext,
        msg: &Value,
    ) -> Result<ProcessedMessage, ProcessError> {
        // Resolve the original webhook: may come from `msg` directly or from
        // ctx.metadata["_raw_webhook"] when an upstream processor serialized.
        let webhook = get_webhook_raw(msg, ctx);

        // Extract the `message` object from the webhook.
        let msg_obj = webhook.get("message").cloned().unwrap_or_default();

        let (content, metadata) = match serde_json::from_value::<FeishuMessage>(msg_obj) {
            Ok(raw) => {
                let content = match raw.msg_type.as_str() {
                    "text" => {
                        // content is a JSON string: {"text":"..."}
                        let content_str = raw.content.as_deref().unwrap_or("{}");
                        let parsed: Value = serde_json::from_str(content_str).unwrap_or_default();
                        parsed
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or(content_str)
                            .to_string()
                    }
                    "post" => {
                        // content is a JSON string:
                        // {"title":"...","content":[[...]]}
                        let content_str = raw.content.as_deref().unwrap_or("{}");
                        post_to_markdown(content_str)
                    }
                    _ => return Err(ProcessError::UnsupportedMessageType(raw.msg_type.clone())),
                };

                let feishu_thread_id = raw.thread_id.or(raw.root_id).or(raw.parent_id);

                let mut metadata = std::collections::BTreeMap::new();
                if let Some(tid) = feishu_thread_id {
                    metadata.insert("feishu_thread_id".to_string(), tid);
                }

                (content, metadata)
            }
            Err(_) => {
                // Not a valid Feishu message object — pass through
                // the raw webhook content field if present.
                let content = webhook
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (content, std::collections::BTreeMap::new())
            }
        };

        // Preserve upstream metadata
        let mut merged = ctx.metadata.clone();
        merged.extend(metadata);

        Ok(ProcessedMessage {
            content,
            metadata: merged,
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve the original webhook JSON from `msg` or ctx metadata.
fn get_webhook_raw(msg: &Value, ctx: &MessageContext) -> Value {
    if msg.get("sender").is_some() || msg.get("message").is_some() {
        return msg.clone();
    }
    if let Some(wh) = ctx.metadata.get("_raw_webhook") {
        if let Ok(parsed) = serde_json::from_str::<Value>(wh) {
            return parsed;
        }
    }
    msg.clone()
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> MessageContext {
        MessageContext::default()
    }

    fn text_webhook(text: &str) -> Value {
        serde_json::json!({
            "sender": { "sender_id": { "open_id": "ou_123" } },
            "message": {
                "chat_id": "oc_chat",
                "chat_type": "p2p",
                "message_type": "text",
                "content": format!("{{\"text\":\"{}\"}}", text)
            }
        })
    }

    fn post_webhook(title: &str, text: &str) -> Value {
        let content_json = serde_json::json!({
            "title": title,
            "content": [[{"tag":"text","text":text,"style":[]}]]
        });
        serde_json::json!({
            "sender": { "sender_id": { "open_id": "ou_123" } },
            "message": {
                "chat_id": "oc_chat",
                "chat_type": "p2p",
                "message_type": "post",
                "content": content_json.to_string()
            }
        })
    }

    #[tokio::test]
    async fn test_text_message() {
        let parser = FeishuParser::new();
        let msg = text_webhook("Hello world");
        let result = parser.process(&make_ctx(), &msg).await.unwrap();
        assert_eq!(result.content, "Hello world");
    }

    #[tokio::test]
    async fn test_post_message() {
        let parser = FeishuParser::new();
        let msg = post_webhook("Test", "hello");
        let result = parser.process(&make_ctx(), &msg).await.unwrap();
        assert!(result.content.contains("## Test"));
        assert!(result.content.contains("hello"));
    }

    #[tokio::test]
    async fn test_thread_id_extraction() {
        let parser = FeishuParser::new();
        let msg = serde_json::json!({
            "sender": { "sender_id": { "open_id": "ou_123" } },
            "message": {
                "chat_id": "oc_chat",
                "chat_type": "p2p",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}",
                "thread_id": "t1",
                "root_id": "r1",
                "parent_id": "p1"
            }
        });
        let result = parser.process(&make_ctx(), &msg).await.unwrap();
        assert_eq!(result.metadata.get("feishu_thread_id").unwrap(), "t1");
    }

    #[tokio::test]
    async fn test_thread_id_fallback_root() {
        let parser = FeishuParser::new();
        let msg = serde_json::json!({
            "sender": { "sender_id": { "open_id": "ou_123" } },
            "message": {
                "chat_id": "oc_chat",
                "chat_type": "p2p",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}",
                "root_id": "r1"
            }
        });
        let result = parser.process(&make_ctx(), &msg).await.unwrap();
        assert_eq!(result.metadata.get("feishu_thread_id").unwrap(), "r1");
    }

    #[tokio::test]
    async fn test_thread_id_fallback_parent() {
        let parser = FeishuParser::new();
        let msg = serde_json::json!({
            "sender": { "sender_id": { "open_id": "ou_123" } },
            "message": {
                "chat_id": "oc_chat",
                "chat_type": "p2p",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}",
                "parent_id": "p1"
            }
        });
        let result = parser.process(&make_ctx(), &msg).await.unwrap();
        assert_eq!(result.metadata.get("feishu_thread_id").unwrap(), "p1");
    }

    #[tokio::test]
    async fn test_unknown_msg_type_returns_error() {
        let parser = FeishuParser::new();
        // message_type "image" should trigger UnsupportedMessageType error
        let msg = serde_json::json!({
            "sender": { "sender_id": { "open_id": "ou_123" } },
            "message": {
                "chat_id": "oc_chat",
                "chat_type": "p2p",
                "message_type": "image",
                "content": "{}"
            }
        });
        let result = parser.process(&make_ctx(), &msg).await;
        assert!(matches!(
            result,
            Err(ProcessError::UnsupportedMessageType(_))
        ));
    }

    #[tokio::test]
    async fn test_no_message_field_passthrough() {
        let parser = FeishuParser::new();
        let msg = serde_json::json!({
            "content": "fallback text"
        });
        let result = parser.process(&make_ctx(), &msg).await.unwrap();
        assert_eq!(result.content, "fallback text");
    }

    #[test]
    fn test_priority() {
        let parser = FeishuParser::new();
        assert_eq!(parser.priority(), 25);
        assert_eq!(parser.phase(), ProcessPhase::Inbound);
    }

    // --- post_to_markdown tests (migrated from content_normalizer) ---

    #[test]
    fn test_post_to_markdown_simple() {
        let post = r###"{"title":"","content":[[{"tag":"text","text":"## 不使用富文本","style":[]}],[{"tag":"text","text":"1. 第一项","style":[]}],[{"tag":"text","text":"2. 第二项","style":[]}],[{"tag":"text","text":"3. 第三项","style":[]}],[],[{"tag":"text","text":"## 使用富文本","style":[]}],[{"tag":"text","text":"1. ","style":[]}],[{"tag":"text","text":"第一项（一级有序列表）","style":[]}]]}"###;
        let md = post_to_markdown(post);
        assert!(md.contains("## 不使用富文本"));
        assert!(md.contains("1. 第一项"));
        assert!(md.contains("第一项（一级有序列表）"));
    }

    #[test]
    fn test_post_to_markdown_styles() {
        let post = r#"{"title":"","content":[[{"tag":"text","text":"普通","style":[]}],[{"tag":"text","text":"加粗","style":["bold"]}],[{"tag":"text","text":"删除线","style":["lineThrough"]}],[{"tag":"text","text":"下划线","style":["underline"]}],[{"tag":"text","text":"加粗下划线","style":["underline","bold"]}],[{"tag":"text","text":"删除线+下划线","style":["lineThrough","underline"]}],[{"tag":"text","text":"加粗+删除线+下划线","style":["lineThrough","underline","bold"]}],[{"tag":"text","text":"引用","style":[]}]]}"#;
        let md = post_to_markdown(post);
        let lines: Vec<&str> = md.lines().collect();
        assert_eq!(lines[0], "普通");
        assert_eq!(lines[1], "**加粗**");
        assert_eq!(lines[2], "~~删除线~~");
        assert_eq!(lines[3], "<u>下划线</u>");
        assert_eq!(lines[4], "**<u>加粗下划线</u>**");
        assert_eq!(lines[5], "~~<u>删除线+下划线</u>~~");
        assert_eq!(lines[6], "**<u>~~加粗+删除线+下划线~~</u>**");
        assert_eq!(lines[7], "引用");
    }

    #[test]
    fn test_post_to_markdown_img() {
        let post = r#"{"title":"","content":[[{"tag":"text","text":"下面是一张图片","style":[]}],[{"tag":"img","image_key":"img_REDACTED","width":1451,"height":597}]]}"#;
        let md = post_to_markdown(post);
        assert!(md.contains("[图片]"));
    }

    #[test]
    fn test_post_to_markdown_at() {
        let post = r#"{"title":"","content":[[{"tag":"at","user_name":"张三","user_id":"ou_123","text":"","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "@张三");
    }

    #[test]
    fn test_post_to_markdown_at_no_name() {
        let post =
            r#"{"title":"","content":[[{"tag":"at","user_id":"ou_123","text":"","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "@某人");
    }

    #[test]
    fn test_post_to_markdown_link() {
        let post = r#"{"title":"","content":[[{"tag":"link","text":"点击这里","href":"https://example.com","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "[点击这里](https://example.com)");
    }

    #[test]
    fn test_post_to_markdown_link_empty_text() {
        let post = r#"{"title":"","content":[[{"tag":"link","text":"","href":"https://example.com","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "[链接](https://example.com)");
    }

    #[test]
    fn test_post_to_markdown_email() {
        let post = r#"{"title":"","content":[[{"tag":"email","email":"test@example.com","text":"","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "<mailto:test@example.com>");
    }

    #[test]
    fn test_post_to_markdown_phone() {
        let post = r#"{"title":"","content":[[{"tag":"phone","phone_number":"12345678","text":"","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "<tel:12345678>");
    }

    #[test]
    fn test_post_to_markdown_channel_at() {
        let post = r#"{"title":"","content":[[{"tag":"channel_at","text":"General","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "@General");
    }

    #[test]
    fn test_post_to_markdown_video() {
        let post = r#"{"title":"","content":[[{"tag":"video","video_key":"vid_xxx","text":"","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "[视频]");
    }

    #[test]
    fn test_post_to_markdown_audio() {
        let post = r#"{"title":"","content":[[{"tag":"audio","audio_key":"aud_xxx","text":"","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert_eq!(md.trim(), "[音频]");
    }

    #[test]
    fn test_post_to_markdown_title() {
        let post = r#"{"title":"会议纪要","content":[[{"tag":"text","text":"正文","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert!(md.starts_with("## 会议纪要"));
    }

    #[test]
    fn test_post_to_markdown_empty_paragraph() {
        let post = r###"{"title":"","content":[[{"tag":"text","text":"第一段","style":[]}],[],[{"tag":"text","text":"第二段","style":[]}]]}"###;
        let md = post_to_markdown(post);
        assert!(md.contains("第一段\n\n第二段"));
    }

    #[test]
    fn test_post_to_markdown_combined_styles() {
        let segs = r#"[[{"tag":"text","text":"加粗下划线","style":["underline","bold"]}]]"#;
        let post = format!(r#"{{"title":"","content":{}}}"#, segs);
        let md = post_to_markdown(&post);
        assert_eq!(md.trim(), "**<u>加粗下划线</u>**");
    }

    #[test]
    fn test_post_to_markdown_invalid_json() {
        let result = post_to_markdown("not json");
        assert_eq!(result, "not json");
    }

    #[test]
    fn test_post_to_markdown_unknown_tag() {
        let post =
            r#"{"title":"","content":[[{"tag":"unknown_tag","text":"some text","style":[]}]]}"#;
        let md = post_to_markdown(post);
        assert!(md.contains("some text"));
    }
}
