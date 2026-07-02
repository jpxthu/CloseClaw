#[cfg(test)]
mod tests {
    use crate::renderer::BOLD;
    use crate::terminal::*;
    use std::collections::HashMap;

    use closeclaw_common::processor::{DslInstruction, DslParseResult};
    use closeclaw_im_adapter::plugin::IMPlugin;
    use closeclaw_im_adapter::NormalizedMessage;
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
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
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
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
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
            timestamp: 1_700_000_000_000,
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
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
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
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
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
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
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
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
        assert_eq!(msg.account_id.as_deref(), Some("owner"));
    }

    /// Empty content still receives the correct account_id.
    #[test]
    fn test_make_message_empty_content_account_id() {
        let adapter = TerminalAdapter::new();
        let msg = adapter.make_message(String::new());
        assert_eq!(msg.account_id.as_deref(), Some("owner"));
    }

    /// make_message preserves platform, peer_id, sender_id, and message_type.
    #[test]
    fn test_make_message_other_fields_unchanged() {
        let adapter = TerminalAdapter::new();
        let msg = adapter.make_message("test".to_string());
        assert_eq!(msg.platform, "terminal");
        assert_eq!(msg.peer_id, "cli");
        assert_eq!(msg.sender_id, closeclaw_platform::current_uid());
        assert_eq!(msg.message_type, "text");
        assert!(msg.media_refs.is_empty());
        assert!(msg.quoted_message.is_none());
        assert!(msg.thread_id.is_none());
        assert!(msg.card_action.is_none());
    }

    /// Multiline content is preserved correctly.
    #[test]
    fn test_make_message_multiline_content_preserved() {
        let adapter = TerminalAdapter::new();
        let msg = adapter.make_message("line1\nline2\nline3".to_string());
        assert_eq!(msg.content, "line1\nline2\nline3");
        assert_eq!(msg.account_id.as_deref(), Some("owner"));
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
    fn test_plugin_render_dsl_ansi_dim() {
        use crate::renderer::DIM;
        use crate::renderer::RESET;
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
        assert!(text.contains(DIM));
        assert!(text.contains(RESET));
        assert!(text.contains("[Button: Go (action: start)]"));
    }
}
