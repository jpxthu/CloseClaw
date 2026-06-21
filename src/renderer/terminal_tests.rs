#[cfg(test)]
mod tests {
    use crate::im_adapter::renderer::Renderer;
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

    // =========================================================================
    // Code block fence tests (Step 1.1)
    // =========================================================================

    /// Unsupported language code block output contains ``` fence (ANSI mode).
    #[test]
    fn test_code_block_unsupported_lang_fence_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("```haskell\nlet x = 1\n```".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        // fence should appear (stripped ANSI sequences won't have ```)
        assert!(
            text.contains("```"),
            "unsupported language block should contain ``` fence"
        );
    }

    /// Unsupported language code block output contains ``` fence (plain text mode).
    #[test]
    fn test_code_block_unsupported_lang_fence_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("```ruby\nputs \"hi\"\n```".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        // plain text: ``` should be present
        assert!(
            text.contains("```"),
            "unsupported language block in plain text should contain ``` fence"
        );
    }

    /// Supported language code block output does NOT contain extra ``` fence.
    #[test]
    fn test_code_block_supported_lang_no_fence() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("```rust\nfn main() {}\n```".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        // supported lang should NOT have ``` fence markers
        let trimmed_lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        // the first non-empty line should NOT be ```
        assert_ne!(
            trimmed_lines.first().copied(),
            Some("```"),
            "supported language block should not start with ``` fence"
        );
    }

    /// Empty language code block output does NOT contain ``` fence.
    #[test]
    fn test_code_block_empty_lang_no_fence() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("```\nhello\n```".into())];
        let output = r.render(&blocks, None);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        // empty language should NOT have ``` fence (only line numbers)
        let trimmed_lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        assert_ne!(
            trimmed_lines.first().copied(),
            Some("```"),
            "empty language block should not start with ``` fence"
        );
    }

    // =========================================================================
    // Selector DSL rendering tests (Step 1.3)
    // =========================================================================

    /// Verify TerminalRenderer renders DSL Selector instructions as plain-text hints.
    #[test]
    fn test_render_selector_dsl_plain() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Pick an option".into())];
        let dsl = DslParseResult {
            clean_content: String::new(),
            instructions: vec![DslInstruction::Selector {
                label: "Color".to_string(),
                options: vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()],
                action: "select_color".to_string(),
            }],
        };
        let output = r.render(&blocks, Some(&dsl));
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("Pick an option"));
        assert!(
            text.contains("[Selector: Color (options: Red, Green, Blue; action: select_color)]"),
            "DSL selector should appear as plain-text hint in output"
        );
    }

    /// Verify DSL Selector renders with DIM style in ANSI mode.
    #[test]
    fn test_render_selector_dsl_ansi() {
        let r = TerminalRenderer::with_ansi(true);
        let blocks = vec![ContentBlock::Text("Content".into())];
        let dsl = DslParseResult {
            clean_content: String::new(),
            instructions: vec![DslInstruction::Selector {
                label: "Size".to_string(),
                options: vec!["S".to_string(), "M".to_string(), "L".to_string()],
                action: "pick_size".to_string(),
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
        assert!(text.contains(DIM));
        assert!(
            text.contains("[Selector: Size (options: S, M, L; action: pick_size)]"),
            "DSL selector should appear as plain-text hint with DIM style"
        );
    }

    /// Verify DSL Selector with empty options.
    #[test]
    fn test_render_selector_empty_options() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Hello".into())];
        let dsl = DslParseResult {
            clean_content: String::new(),
            instructions: vec![DslInstruction::Selector {
                label: "Choose".to_string(),
                options: vec![],
                action: "pick".to_string(),
            }],
        };
        let output = r.render(&blocks, Some(&dsl));
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(
            text.contains("[Selector: Choose (options: ; action: pick)]"),
            "Selector with empty options should render correctly"
        );
    }

    /// Verify Button and Selector DSL instructions render together.
    #[test]
    fn test_render_button_and_selector_dsl() {
        let r = TerminalRenderer::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Choose:".into())];
        let dsl = DslParseResult {
            clean_content: String::new(),
            instructions: vec![
                DslInstruction::Button {
                    label: "OK".to_string(),
                    action: "confirm".to_string(),
                    value: "yes".to_string(),
                },
                DslInstruction::Selector {
                    label: "Color".to_string(),
                    options: vec!["R".to_string(), "G".to_string()],
                    action: "select".to_string(),
                },
            ],
        };
        let output = r.render(&blocks, Some(&dsl));
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(text.contains("[Button: OK (action: confirm, value: yes)]"));
        assert!(text.contains("[Selector: Color (options: R, G; action: select)]"));
    }
}
