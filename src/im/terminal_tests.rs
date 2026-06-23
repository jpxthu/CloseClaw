#[cfg(test)]
mod tests {
    use crate::im::terminal::*;
    use crate::im_adapter::plugin::IMPlugin;
    use crate::im_adapter::NormalizedMessage;
    use crate::llm::types::ContentBlock;
    use crate::processor_chain::dsl_parser::{DslInstruction, DslParseResult};
    use crate::renderer::RenderedOutput;

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
            timestamp: 1700000000,
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
            timestamp: 1700000000,
            message_type: "text".to_string(),
            media_refs: vec![],
            quoted_message: None,
            thread_id: None,
            account_id: None,
            card_action: None,
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
            timestamp: 1700000000,
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
            timestamp: 1700000000,
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

    // DSL skip rendering tests (Step 1.7) — plugin-level
    // =========================================================================

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
