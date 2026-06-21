#[cfg(test)]
mod tests {
    use crate::im::terminal::*;
    use crate::im_adapter::plugin::IMPlugin;
    use crate::im_adapter::renderer::{RenderedOutput, Renderer};
    use crate::im_adapter::NormalizedMessage;
    use crate::llm::types::ContentBlock;
    use crate::processor_chain::dsl_parser::{DslInstruction, DslParseResult};
    use crate::renderer::terminal::{strip_ansi, TerminalRenderer, BOLD, DIM, ITALIC, RESET};

    #[test]
    fn test_platform() {
        let r = TerminalRenderer::new();
        assert_eq!(r.platform(), "terminal");
    }

    #[test]
    fn test_render_text_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Hello world".into())];
        let output = r.render(&blocks, None);
        assert_eq!(output.msg_type, "text");
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Hello world"));
        assert!(!text.contains("\x1b["));
    }

    #[test]
    fn test_render_text_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("**bold**".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains(BOLD));
        assert!(text.contains("bold"));
    }

    #[test]
    fn test_render_thinking() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Thinking("reasoning here".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[Thinking]"));
        assert!(text.contains("reasoning here"));
        assert!(text.contains("[end of thinking]"));
    }

    #[test]
    fn test_render_tool_use() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "web_search".into(),
            input: r#"{"query":"rust"}"#.into(),
        }];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("⚙"));
        assert!(text.contains("web_search"));
        assert!(text.contains("rust"));
    }

    #[test]
    fn test_render_tool_result_truncation() {
        let r = TerminalRenderer::with_ansi(false);
        let long_content = "x".repeat(200);
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "call_1".into(),
            content: long_content,
        }];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("(truncated)"));
        assert!(text.len() < 200);
    }

    #[test]
    fn test_render_tool_result_short() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "call_1".into(),
            content: "ok".into(),
        }];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("ok"));
        assert!(!text.contains("(truncated)"));
    }

    #[test]
    fn test_render_code_block() {
        let r = TerminalRenderer::with_ansi(false);
        let code = "fn main() {\n    println!(\"hi\");\n}";
        let blocks = vec![ContentBlock::Text(format!("```rust\n{}\n```", code))];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("rust"));
        assert!(text.contains("│"));
        assert!(text.contains("fn main()"));
    }

    #[test]
    fn test_render_code_block_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let code = "let x = 1;";
        let blocks = vec![ContentBlock::Text(format!("```js\n{}\n```", code))];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("js"));
        assert!(text.contains("let x = 1;"));
    }

    #[test]
    fn test_render_heading() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("# Title".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains(BOLD));
        assert!(text.contains("Title"));
    }

    #[test]
    fn test_render_bold() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("This is **bold** text".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains(BOLD));
        assert!(text.contains("bold"));
    }

    #[test]
    fn test_render_link() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text(
            "[Rust](https://rust-lang.org) is great".into(),
        )];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Rust"));
        assert!(text.contains("https://rust-lang.org"));
    }

    #[test]
    fn test_render_hr() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("---".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("───"));
    }

    #[test]
    fn test_render_blockquote() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("> quote".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("│"));
        assert!(text.contains("quote"));
    }

    #[test]
    fn test_strip_ansi() {
        let input = format!("{}hello{}", BOLD, RESET);
        assert_eq!(strip_ansi(&input), "hello");
    }

    #[test]
    fn test_strip_ansi_nested() {
        let input = format!("{}{}nested{}", BOLD, DIM, RESET);
        assert_eq!(strip_ansi(&input), "nested");
    }

    #[test]
    fn test_render_thinking_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Thinking("thinking text".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[Thinking]"));
        assert!(text.contains("thinking text"));
        assert!(text.contains("[end of thinking]"));
        assert!(!text.contains("\x1b["));
    }

    #[test]
    fn test_render_empty_blocks() {
        let r = TerminalRenderer::with_ansi(true);
        let output = r.render(&[], None);
        assert_eq!(output.msg_type, "text");
    }

    #[test]
    fn test_render_mixed_blocks() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![
            ContentBlock::Text("Step 1".into()),
            ContentBlock::ToolUse {
                id: "c1".into(),
                name: "exec".into(),
                input: "ls".into(),
            },
            ContentBlock::ToolResult {
                tool_call_id: "c1".into(),
                content: "file.rs".into(),
            },
            ContentBlock::Text("Done".into()),
        ];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Step 1"));
        assert!(text.contains("exec"));
        assert!(text.contains("file.rs"));
        assert!(text.contains("Done"));
    }

    #[test]
    fn test_render_tool_use_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::ToolUse {
            id: "call_2".into(),
            name: "read_file".into(),
            input: r#"{"path":"/tmp/x"}"#.into(),
        }];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("read_file"));
        assert!(!text.contains("\x1b["));
    }

    #[test]
    fn test_render_italic() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("use *italic* please".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains(ITALIC));
        assert!(text.contains("italic"));
    }

    #[test]
    fn test_render_inline_code() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("use `println!` macro".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("println!"));
    }

    #[test]
    fn test_render_code_block_no_language() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("```\nhello\n```".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("│"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn test_render_code_block_ansi_has_dim_header() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("```python\nprint(1)\n```".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("python"));
        assert!(text.contains(DIM));
    }

    #[test]
    fn test_render_long_tool_result_not_truncated() {
        let r = TerminalRenderer::with_ansi(false);
        let short = "a".repeat(50);
        let blocks = vec![ContentBlock::ToolResult {
            tool_call_id: "c1".into(),
            content: short.clone(),
        }];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains(&short));
        assert!(!text.contains("(truncated)"));
    }

    #[test]
    fn test_render_plain_text_strip_bold() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("**bold**".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("bold"));
        assert!(!text.contains("**"));
        assert!(!text.contains("\x1b["));
    }

    #[test]
    fn test_render_plain_text_strip_italic() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("*italic*".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("italic"));
        assert!(!text.contains("*"));
    }

    #[test]
    fn test_render_plain_text_link() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("[click](https://x.com)".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("click (https://x.com)"));
    }

    #[test]
    fn test_render_plain_text_heading() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("# Hello".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Hello"));
        assert!(!text.contains("#"));
    }

    #[test]
    fn test_render_plain_text_blockquote() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("> quote".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("│ quote"));
    }

    #[test]
    fn test_render_plain_text_hr() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("---".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("───"));
    }

    #[test]
    fn test_render_plain_text_inline_code() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("use `foo` here".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("foo"));
    }

    #[test]
    fn test_payload_is_json() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("test".into())];
        let output = r.render(&blocks, None);
        // payload should be valid JSON
        let s = serde_json::to_string(&output.payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(parsed.get("content").is_some());
    }

    // =========================================================================
    // TerminalAdapter tests
    // =========================================================================

    #[test]
    fn test_adapter_new() {
        let _adapter = TerminalAdapter::new();
    }

    #[test]
    fn test_read_input_returns_none_on_eof() {
        let adapter = TerminalAdapter::new();
        // stdin is empty in test environment -> EOF -> None
        assert!(adapter.read_input().is_none());
    }

    #[test]
    fn test_read_input_blank_lines_only_returns_none() {
        let adapter = TerminalAdapter::new();
        // Leading blank lines are skipped; with no content accumulated -> None
        assert!(adapter.read_input().is_none());
    }

    #[test]
    fn test_normalized_message_platform_and_peer() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "hello".to_string(),
            timestamp: 1700000000,
            thread_id: None,
            account_id: None,
        };
        assert_eq!(msg.platform, "terminal");
        assert_eq!(msg.peer_id, "cli");
        assert_eq!(msg.content, "hello");
    }

    #[test]
    fn test_normalized_message_optional_fields_none() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "test".to_string(),
            timestamp: 1700000000,
            thread_id: None,
            account_id: None,
        };
        assert!(msg.thread_id.is_none());
        assert!(msg.account_id.is_none());
    }

    #[test]
    fn test_normalized_message_timestamp_is_reasonable() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "test".to_string(),
            timestamp: 1700000000,
            thread_id: None,
            account_id: None,
        };
        // Timestamp is a valid Unix timestamp (after 2023)
        assert!(msg.timestamp > 1672531200);
    }

    #[test]
    fn test_normalized_message_serialization_roundtrip() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "hello\nworld".to_string(),
            timestamp: 1700000000,
            thread_id: None,
            account_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: NormalizedMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.platform, "terminal");
        assert_eq!(deserialized.content, "hello\nworld");
    }

    #[test]
    fn test_normalized_message_empty_content() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: String::new(),
            timestamp: 1700000000,
            thread_id: None,
            account_id: None,
        };
        assert!(msg.content.is_empty());
    }

    #[test]
    fn test_normalized_message_multiline_content() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "line1\nline2\nline3".to_string(),
            timestamp: 1700000000,
            thread_id: None,
            account_id: None,
        };
        let lines: Vec<&str> = msg.content.lines().collect();
        assert_eq!(lines.len(), 3);
    }

    // =========================================================================
    // TerminalPlugin tests
    // =========================================================================

    #[test]
    fn test_plugin_platform_returns_terminal() {
        let plugin = TerminalPlugin::new();
        assert_eq!(plugin.platform(), "terminal");
    }

    #[test]
    fn test_plugin_with_ansi_platform() {
        let plugin = TerminalPlugin::with_ansi(true);
        assert_eq!(plugin.platform(), "terminal");
    }

    #[test]
    fn test_plugin_default() {
        let plugin = TerminalPlugin::default();
        assert_eq!(plugin.platform(), "terminal");
    }

    #[tokio::test]
    async fn test_plugin_parse_inbound_eof() {
        let plugin = TerminalPlugin::new();
        let result = plugin.parse_inbound(b"").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_plugin_parse_inbound_none_with_ansi() {
        let plugin = TerminalPlugin::with_ansi(false);
        let result = plugin.parse_inbound(b"").await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_plugin_render_delegates_to_renderer() {
        let plugin = TerminalPlugin::with_ansi(false);
        let blocks = vec![ContentBlock::Text("hello world".into())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "text");
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("hello world"));
    }

    #[test]
    fn test_plugin_render_with_ansi() {
        let plugin = TerminalPlugin::with_ansi(true);
        let blocks = vec![ContentBlock::Text("**bold**".into())];
        let output = plugin.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains(BOLD));
    }

    #[test]
    fn test_plugin_render_empty_blocks() {
        let plugin = TerminalPlugin::new();
        let output = plugin.render(&[], None);
        assert_eq!(output.msg_type, "text");
    }

    #[test]
    fn test_plugin_render_mixed_content() {
        let plugin = TerminalPlugin::with_ansi(false);
        let blocks = vec![
            ContentBlock::Text("first".into()),
            ContentBlock::ToolUse {
                id: "c1".into(),
                name: "exec".into(),
                input: "ls".into(),
            },
            ContentBlock::ToolResult {
                tool_call_id: "c1".into(),
                content: "ok".into(),
            },
        ];
        let output = plugin.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("first"));
        assert!(text.contains("exec"));
        assert!(text.contains("ok"));
    }

    #[tokio::test]
    async fn test_plugin_send_ok() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({
                "content": { "text": "test output" }
            }),
        };
        let result = plugin.send(&output, "cli", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_plugin_send_empty_text() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({
                "content": { "text": "" }
            }),
        };
        let result = plugin.send(&output, "cli", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_plugin_send_null_payload() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::Null,
        };
        let result = plugin.send(&output, "cli", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_plugin_send_missing_content_key() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({}),
        };
        let result = plugin.send(&output, "cli", None).await;
        assert!(result.is_ok());
    }

    // =========================================================================
    // ContentBlock placeholder rendering tests (Step 1.1)
    // =========================================================================

    #[test]
    fn test_render_image_placeholder_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Image("photo.jpg".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[image: photo.jpg]"));
        assert!(!text.contains("\x1b["));
    }

    #[test]
    fn test_render_image_placeholder_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Image("screenshot.png".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[image: screenshot.png]"));
        assert!(text.contains(DIM));
    }

    #[test]
    fn test_render_audio_placeholder_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Audio("voice.mp3".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[audio: voice.mp3]"));
        assert!(!text.contains("\x1b["));
    }

    #[test]
    fn test_render_audio_placeholder_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Audio("recording.wav".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[audio: recording.wav]"));
        assert!(text.contains(DIM));
    }

    #[test]
    fn test_render_file_placeholder_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::File("document.pdf".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[file: document.pdf]"));
        assert!(!text.contains("\x1b["));
    }

    #[test]
    fn test_render_file_placeholder_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::File("data.csv".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[file: data.csv]"));
        assert!(text.contains(DIM));
    }

    #[test]
    fn test_render_placeholder_mixed_with_text() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![
            ContentBlock::Text("Here is an image:".into()),
            ContentBlock::Image("chart.png".into()),
            ContentBlock::Text("And a file:".into()),
            ContentBlock::File("report.pdf".into()),
        ];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Here is an image:"));
        assert!(text.contains("[image: chart.png]"));
        assert!(text.contains("And a file:"));
        assert!(text.contains("[file: report.pdf]"));
    }

    #[tokio::test]
    async fn test_plugin_send_with_thread_id() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({
                "content": { "text": "thread reply" }
            }),
        };
        let result = plugin.send(&output, "cli", Some("thread_123")).await;
        assert!(result.is_ok());
    }

    // =========================================================================
    // DSL skip rendering tests (Step 1.7)
    // =========================================================================

    /// Verify TerminalRenderer renders DSL Button instructions as plain-text hints.
    #[test]
    fn test_render_dsl_not_in_output() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Hello".into())];
        let dsl = DslParseResult {
            clean_content: String::new(),
            instructions: vec![DslInstruction::Button {
                label: "Click Me".to_string(),
                action: "navigate".to_string(),
                value: "/home".to_string(),
            }],
        };
        let output = r.render(&blocks, Some(&dsl));
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Hello"));
        assert!(
            text.contains("[Button: Click Me (action: navigate, value: /home)]"),
            "DSL button should appear as plain-text hint in output"
        );
    }

    /// Verify DSL Button renders as plain-text hint even in ANSI mode.
    #[test]
    fn test_render_dsl_not_in_output_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("Content".into())];
        let dsl = DslParseResult {
            clean_content: String::new(),
            instructions: vec![DslInstruction::Button {
                label: "OK".to_string(),
                action: "confirm".to_string(),
                value: "yes".to_string(),
            }],
        };
        let output = r.render(&blocks, Some(&dsl));
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Content"));
        assert!(
            text.contains("[Button: OK (action: confirm, value: yes)]"),
            "DSL button should appear as plain-text hint in output"
        );
    }

    /// Verify empty DSL instructions produce no visible output.
    #[test]
    fn test_render_empty_dsl_no_output() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Hello world".into())];
        let dsl = DslParseResult {
            clean_content: "Hello world".to_string(),
            instructions: vec![],
        };
        let output = r.render(&blocks, Some(&dsl));
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Hello world"));
    }

    /// Verify DSL Button text appears in plugin-level render output.
    #[test]
    fn test_plugin_render_dsl_not_in_output() {
        let plugin = TerminalPlugin::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Some text".into())];
        let dsl = DslParseResult {
            clean_content: String::new(),
            instructions: vec![DslInstruction::Button {
                label: "Click".to_string(),
                action: "go".to_string(),
                value: "ok".to_string(),
            }],
        };
        let output = plugin.render(&blocks, Some(&dsl));
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Some text"));
        assert!(
            text.contains("[Button: Click (action: go, value: ok)]"),
            "DSL button should appear as plain-text hint in output"
        );
    }
}
