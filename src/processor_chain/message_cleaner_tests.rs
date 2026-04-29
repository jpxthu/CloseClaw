//! MessageCleaner tests

#[cfg(test)]
mod tests {
    use crate::processor_chain::context::MessageContext;
    use crate::processor_chain::message_cleaner::post_to_markdown;
    use crate::processor_chain::processor::MessageProcessor;
    use crate::processor_chain::MessageCleaner;

    // -------------------------------------------------------------------------
    // post_to_markdown
    // -------------------------------------------------------------------------

    #[test]
    fn test_post_to_markdown_simple() {
        let post = r###"{"title":"","content":[[{"tag":"text","text":"## 不使用富文本","style":[]}],[{"tag":"text","text":"1. 第一项","style":[]}],[{"tag":"text","text":"2. 第二项","style":[]}],[{"tag":"text","text":"3. 第三项","style":[]}],[],[{"tag":"text","text":"## 使用富文本","style":[]}],[{"tag":"text","text":"1. ","style":[]},{"tag":"text","text":"第一项（一级有序列表）","style":[]}]]}"###;
        let md = post_to_markdown(post);
        assert!(md.contains("## 不使用富文本"));
        assert!(md.contains("1. 第一项"));
        assert!(md.contains("1. 第一项（一级有序列表）"));
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
        let post = r#"{"title":"","content":[[{"tag":"text","text":"下面是一张图片，内容是刚才一条包含很多表情的消息的截图：","style":[]}],[{"tag":"img","image_key":"img_REDACTED","width":1451,"height":597}]]}"#;
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

    // -------------------------------------------------------------------------
    // MessageCleaner process()
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_process_text_simple() {
        let cleaner = MessageCleaner::new();
        let raw = crate::processor_chain::context::RawMessage {
            platform: "feishu".to_string(),
            sender_id: "ou_123".to_string(),
            content: r#"{"msg_type":"text","text":{"text":"Hello world"}}"#.to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "om_abc".to_string(),
        };
        let ctx = MessageContext::from_raw(raw);
        let result = cleaner.process(&ctx).await.unwrap();
        assert!(result.is_some());
        let out = result.unwrap();
        assert_eq!(out.content, "Hello world");
        assert!(out.metadata.get("feishu_thread_id").is_none());
    }

    #[tokio::test]
    async fn test_process_feishu_thread_id() {
        let cleaner = MessageCleaner::new();
        let raw = crate::processor_chain::context::RawMessage {
            platform: "feishu".to_string(),
            sender_id: "ou_123".to_string(),
            content: r#"{"msg_type":"text","text":{"text":"在话题中回复的消息"},"thread_id":"omt_1a8b5a3fbe4ddbee","root_id":"om_x100b51d961a4088cc42c33abed8140f","parent_id":"om_x100b51d961a4088cc42c33abed8140f"}"#.to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "om_x100b51d903f718b0c4f4598c36ff2e7".to_string(),
        };
        let ctx = MessageContext::from_raw(raw);
        let result = cleaner.process(&ctx).await.unwrap();
        assert!(result.is_some());
        let out = result.unwrap();
        assert_eq!(out.content, "在话题中回复的消息");
        assert_eq!(
            out.metadata
                .get("feishu_thread_id")
                .unwrap()
                .as_str()
                .unwrap(),
            "omt_1a8b5a3fbe4ddbee"
        );
    }

    #[tokio::test]
    async fn test_process_feishu_root_id_fallback() {
        let cleaner = MessageCleaner::new();
        let raw = crate::processor_chain::context::RawMessage {
            platform: "feishu".to_string(),
            sender_id: "ou_456".to_string(),
            content:
                r#"{"msg_type":"text","text":{"text":"reply to root"},"root_id":"om_root_123"}"#
                    .to_string(),
            timestamp: chrono::Utc::now(),
            message_id: "om_reply_456".to_string(),
        };
        let ctx = MessageContext::from_raw(raw);
        let result = cleaner.process(&ctx).await.unwrap();
        assert!(result.is_some());
        let out = result.unwrap();
        assert_eq!(
            out.metadata
                .get("feishu_thread_id")
                .unwrap()
                .as_str()
                .unwrap(),
            "om_root_123"
        );
    }
}
