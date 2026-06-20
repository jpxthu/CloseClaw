use super::*;

fn make_ctx() -> MessageContext {
    MessageContext::default()
}

fn text_webhook(text: &str) -> serde_json::Value {
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

fn post_webhook(title: &str, text: &str) -> serde_json::Value {
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
    let post = r#"{"title":"","content":[[{"tag":"at","user_id":"ou_123","text":"","style":[]}]]}"#;
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
    let post =
        r#"{"title":"","content":[[{"tag":"video","video_key":"vid_xxx","text":"","style":[]}]]}"#;
    let md = post_to_markdown(post);
    assert_eq!(md.trim(), "[视频]");
}

#[test]
fn test_post_to_markdown_audio() {
    let post =
        r#"{"title":"","content":[[{"tag":"audio","audio_key":"aud_xxx","text":"","style":[]}]]}"#;
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
    let post = r#"{"title":"","content":[[{"tag":"unknown_tag","text":"some text","style":[]}]]}"#;
    let md = post_to_markdown(post);
    assert!(md.contains("some text"));
}
