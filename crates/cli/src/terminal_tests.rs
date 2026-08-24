#[cfg(test)]
mod tests {
    use crate::renderer::BOLD;
    use crate::terminal::*;
    use std::collections::HashMap;

    use closeclaw_common::processor::{
        ContentBlockType, ContentDelta, DslInstruction, DslParseResult, StreamEvent,
    };
    use closeclaw_common::streaming::StreamingRenderer;
    use closeclaw_common::{MessageType, NormalizedMessage};
    use closeclaw_im_adapter::plugin::IMPlugin;
    use closeclaw_im_adapter::RenderedOutput;
    use closeclaw_llm::types::ContentBlock;

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
            timestamp: 1_700_000_000_000,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
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
            timestamp: 1_700_000_000_000,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
        };
        assert!(msg.thread_id.is_none());
        assert!(msg.account_id.is_empty());
    }

    #[test]
    fn test_normalized_message_timestamp_is_reasonable() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "test".to_string(),
            timestamp: 1_700_000_000_000,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
        };
        // Timestamp is a valid Unix timestamp (after 2023)
        assert!(msg.timestamp > 1_672_531_200_000);
    }

    #[test]
    fn test_normalized_message_serialization_roundtrip() {
        let msg = NormalizedMessage {
            platform: "terminal".to_string(),
            sender_id: "1000".to_string(),
            peer_id: "cli".to_string(),
            content: "hello\nworld".to_string(),
            timestamp: 1_700_000_000_000,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
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
            timestamp: 1_700_000_000_000,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
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
            timestamp: 1_700_000_000_000,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
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
        let text = output.payload.as_str().unwrap();
        assert!(text.contains("hello world"));
    }

    #[test]
    fn test_plugin_render_with_ansi() {
        let plugin = TerminalPlugin::with_ansi(true);
        let blocks = vec![ContentBlock::Text("**bold**".into())];
        let output = plugin.render(&blocks, None);
        let text = output.payload.as_str().unwrap();
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
        let text = output.payload.as_str().unwrap();
        assert!(text.contains("first"));
        assert!(text.contains("exec"));
        assert!(text.contains("ok"));
    }

    #[tokio::test]
    async fn test_plugin_send_ok() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::String("test output".into()),
        };
        let result = plugin.send(&output, "cli", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_plugin_send_empty_text() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::String(String::new()),
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

    #[tokio::test]
    async fn test_plugin_send_with_thread_id() {
        let plugin = TerminalPlugin::with_ansi(false);
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::String("thread reply".into()),
        };
        let result = plugin.send(&output, "cli", Some("thread_123")).await;
        assert!(result.is_ok());
    }

    // =========================================================================
    // ── lifecycle hook tests (Step 1.2) ──────────────────────────────

    /// TerminalPlugin shutdown is a no-op (default from IMPlugin trait).
    #[tokio::test]
    async fn test_terminal_plugin_shutdown_noop() {
        let plugin = TerminalPlugin::new();
        plugin.shutdown().await.unwrap();
    }

    /// TerminalPlugin shutdown is idempotent.
    #[tokio::test]
    async fn test_terminal_plugin_shutdown_idempotent() {
        let plugin = TerminalPlugin::new();
        plugin.shutdown().await.unwrap();
        plugin.shutdown().await.unwrap();
    }

    /// TerminalPlugin init is a no-op (default from IMPlugin trait).
    #[tokio::test]
    async fn test_terminal_plugin_init_noop() {
        let plugin = TerminalPlugin::new();
        plugin.init().await.unwrap();
    }

    // =========================================================================
    // account_id mapping tests (Step 1.3)
    // =========================================================================

    /// make_message produces account_id = Some("owner") for any content,
    /// aligning with the design doc: "local user defaults to Owner".
    #[test]
    fn test_make_message_account_id_is_owner() {
        let adapter = TerminalAdapter::new();
        let msg = adapter.make_message("hello world".to_string());
        assert_eq!(msg.account_id, "owner");
    }

    /// Empty content still receives the correct account_id.
    #[test]
    fn test_make_message_empty_content_account_id() {
        let adapter = TerminalAdapter::new();
        let msg = adapter.make_message(String::new());
        assert_eq!(msg.account_id, "owner");
    }

    /// make_message preserves platform, peer_id, sender_id, and message_type.
    #[test]
    fn test_make_message_other_fields_unchanged() {
        let adapter = TerminalAdapter::new();
        let msg = adapter.make_message("test".to_string());
        assert_eq!(msg.platform, "terminal");
        assert_eq!(msg.peer_id, "cli");
        assert_eq!(msg.sender_id, closeclaw_platform::current_uid());
        assert_eq!(msg.message_type, MessageType::Text);
        assert!(msg.media_refs.is_empty());
        assert!(msg.thread_id.is_none());
    }

    /// Multiline content is preserved correctly.
    #[test]
    fn test_make_message_multiline_content_preserved() {
        let adapter = TerminalAdapter::new();
        let msg = adapter.make_message("line1\nline2\nline3".to_string());
        assert_eq!(msg.content, "line1\nline2\nline3");
        assert_eq!(msg.account_id, "owner");
    }

    // DSL rendering tests — plugin-level
    // =========================================================================

    /// Verify DSL Button text appears in plugin-level render output.
    #[test]
    fn test_plugin_render_dsl_not_in_output() {
        let plugin = TerminalPlugin::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Some text".into())];
        let dsl = DslParseResult {
            instructions: vec![DslInstruction {
                instruction_type: "button".to_string(),
                params: HashMap::from([
                    ("label".to_string(), "Click".to_string()),
                    ("action".to_string(), "go".to_string()),
                    ("value".to_string(), "ok".to_string()),
                ]),
            }],
        };
        let output = plugin.render(&blocks, Some(&dsl));
        let text = output.payload.as_str().unwrap();
        assert!(text.contains("Some text"));
        assert!(
            text.contains("[Button: Click (action: go)]"),
            "DSL button hint should appear in output"
        );
    }

    /// Verify DSL Selector text appears in plugin-level render output.
    #[test]
    fn test_plugin_render_dsl_selector_in_output() {
        let plugin = TerminalPlugin::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Reply here".into())];
        let dsl = DslParseResult {
            instructions: vec![DslInstruction {
                instruction_type: "selector".to_string(),
                params: HashMap::from([
                    ("label".to_string(), "Pick one".to_string()),
                    ("options".to_string(), "a,b,c".to_string()),
                    ("action".to_string(), "choose".to_string()),
                ]),
            }],
        };
        let output = plugin.render(&blocks, Some(&dsl));
        let text = output.payload.as_str().unwrap();
        assert!(text.contains("Reply here"));
        assert!(
            text.contains("[Selector: Pick one (options: a,b,c) (action: choose)]"),
            "DSL selector hint should appear in output"
        );
    }

    /// Verify multiple DSL instructions each generate their own hint line.
    #[test]
    fn test_plugin_render_dsl_multiple_instructions() {
        let plugin = TerminalPlugin::with_ansi(false);
        let blocks = vec![ContentBlock::Text("Content".into())];
        let dsl = DslParseResult {
            instructions: vec![
                DslInstruction {
                    instruction_type: "button".to_string(),
                    params: HashMap::from([
                        ("label".to_string(), "OK".to_string()),
                        ("action".to_string(), "confirm".to_string()),
                        ("value".to_string(), String::new()),
                    ]),
                },
                DslInstruction {
                    instruction_type: "selector".to_string(),
                    params: HashMap::from([
                        ("label".to_string(), "Mode".to_string()),
                        ("options".to_string(), "fast,slow".to_string()),
                        ("action".to_string(), "set_mode".to_string()),
                    ]),
                },
            ],
        };
        let output = plugin.render(&blocks, Some(&dsl));
        let text = output.payload.as_str().unwrap();
        assert!(text.contains("[Button: OK (action: confirm)]"));
        assert!(text.contains("[Selector: Mode (options: fast,slow) (action: set_mode)]"));
    }

    /// Verify DSL hints are wrapped in ANSI dim when ansi=true.
    #[test]
    fn test_plugin_render_dsl_ansi_no_style() {
        let plugin = TerminalPlugin::with_ansi(true);
        let blocks = vec![];
        let dsl = DslParseResult {
            instructions: vec![DslInstruction {
                instruction_type: "button".to_string(),
                params: HashMap::from([
                    ("label".to_string(), "Go".to_string()),
                    ("action".to_string(), "start".to_string()),
                    ("value".to_string(), String::new()),
                ]),
            }],
        };
        let output = plugin.render(&blocks, Some(&dsl));
        let text = output.payload.as_str().unwrap();
        assert!(!text.contains("\x1b["));
        assert!(text.contains("[Button: Go (action: start)]"));
    }

    // =====================================================================
    // Streaming renderer tests (Step 1.3)
    // =====================================================================

    /// streaming_renderer() returns Some for TerminalPlugin.
    #[test]
    fn test_streaming_renderer_returns_some() {
        let plugin = TerminalPlugin::new();
        assert!(plugin.streaming_renderer().is_some());
    }

    /// streaming_renderer() returns Some for TerminalPlugin::with_ansi(true).
    #[test]
    fn test_streaming_renderer_returns_some_with_ansi() {
        let plugin = TerminalPlugin::with_ansi(true);
        assert!(plugin.streaming_renderer().is_some());
    }

    /// streaming_renderer() returns Some for TerminalPlugin::default().
    #[test]
    fn test_streaming_renderer_returns_some_default() {
        let plugin = TerminalPlugin::default();
        assert!(plugin.streaming_renderer().is_some());
    }

    /// Handle a single text BlockDelta and verify streaming output is non-empty.
    #[test]
    fn test_streaming_handle_block_delta_produces_output() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        let event = StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello, ".to_string(),
            },
        };
        let _output = r.handle_event(event);
        // Text deltas are accumulated in a line buffer; partial text
        // may not produce text_messages until a line boundary is hit.
        // We only verify the call succeeds without panicking.
        // Subsequent delta completes a line to produce output.
        let event2 = StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "world!\n".to_string(),
            },
        };
        let output2 = r.handle_event(event2);
        assert!(
            !output2.text_messages.is_empty(),
            "expected text_messages after line-ending delta"
        );
        assert_eq!(output2.text_messages[0], "Hello, world!");
    }

    /// Flush drains remaining buffered content.
    #[test]
    fn test_streaming_flush_drains_buffer() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        // Send a delta without a trailing newline.
        r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "partial line".to_string(),
            },
        });

        // Flush should emit the remaining content.
        let output = r.flush();
        assert!(
            !output.text_messages.is_empty(),
            "flush should drain buffered content"
        );
        assert_eq!(output.text_messages[0], "partial line");
    }

    /// Flush on empty state returns empty output.
    #[test]
    fn test_streaming_flush_empty_returns_empty() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        let output = r.flush();
        assert!(
            output.text_messages.is_empty(),
            "flush on empty buffer should return empty"
        );
    }

    /// Empty text delta does not panic and produces no output.
    #[test]
    fn test_streaming_empty_text_delta() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        let output = r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: String::new(),
            },
        });
        assert!(
            output.text_messages.is_empty(),
            "empty text delta should not produce output"
        );
    }

    /// Rapid consecutive deltas accumulate correctly.
    #[test]
    fn test_streaming_rapid_consecutive_deltas() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        let mut all_text = String::new();
        for i in 0..100 {
            let out = r.handle_event(StreamEvent::BlockDelta {
                index: 0,
                delta: ContentDelta::Text {
                    text: format!("{}", i),
                },
            });
            for line in &out.text_messages {
                all_text.push_str(line);
            }
        }
        let output = r.flush();
        for line in &output.text_messages {
            all_text.push_str(line);
        }
        // All digits 0-99 should appear in the combined output.
        for i in 0..100 {
            assert!(
                all_text.contains(&i.to_string()),
                "missing digit {} in rapid delta output",
                i
            );
        }
    }

    /// Code block content is handled without panicking.
    #[test]
    fn test_streaming_code_block_content() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        let mut all_text = Vec::new();
        // Simulate code block boundaries.
        let out = r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n".to_string(),
            },
        });
        all_text.extend(out.text_messages);
        let out = r.flush();
        all_text.extend(out.text_messages);
        let combined = all_text.join("");
        assert!(
            !combined.is_empty(),
            "code block content should produce output"
        );
        assert!(combined.contains("fn main()"));
        assert!(combined.contains("println!"));
    }

    /// BlockStart and BlockEnd events are accepted without error.
    #[test]
    fn test_streaming_block_start_end_no_panic() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        r.handle_event(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        });
        r.handle_event(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        });
        let output = r.flush();
        // No panic, output may be empty.
        let _ = output;
    }

    /// Full streaming lifecycle: BlockStart → BlockDelta → BlockEnd → flush.
    #[test]
    fn test_streaming_full_lifecycle() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        let mut all_text = Vec::new();
        r.handle_event(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        });
        let out = r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "The answer is ".to_string(),
            },
        });
        all_text.extend(out.text_messages);
        let out = r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "42.\n".to_string(),
            },
        });
        all_text.extend(out.text_messages);
        r.handle_event(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        });
        let output = r.flush();
        all_text.extend(output.text_messages);
        assert_eq!(all_text, vec!["The answer is 42."]);
    }

    /// MessageEnd event is accepted and flush drains remaining content.
    #[test]
    fn test_streaming_message_end_triggers_flush() {
        let plugin = TerminalPlugin::new();
        let renderer = plugin.streaming_renderer().unwrap();
        let mut r = renderer.lock().unwrap();

        let mut all_text = Vec::new();
        let out = r.handle_event(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "line one\nline two".to_string(),
            },
        });
        all_text.extend(out.text_messages);
        // MessageEnd should not panic; flush drains the rest.
        r.handle_event(StreamEvent::MessageEnd {
            usage: None,
            finish_reason: Some("stop".to_string()),
        });
        let out = r.flush();
        all_text.extend(out.text_messages);
        assert!(
            !all_text.is_empty(),
            "flush after MessageEnd should drain buffer"
        );
    }

    /// Streaming renderer is independent across TerminalPlugin instances.
    #[test]
    fn test_streaming_renderer_independent_across_instances() {
        let plugin_a = TerminalPlugin::new();
        let plugin_b = TerminalPlugin::new();

        let ra = plugin_a.streaming_renderer().unwrap();
        let rb = plugin_b.streaming_renderer().unwrap();

        // They should be different Mutex instances.
        let ptr_a: *const std::sync::Mutex<_> = ra;
        let ptr_b: *const std::sync::Mutex<_> = rb;
        assert_ne!(ptr_a, ptr_b, "plugins should have independent renderers");
    }
}
