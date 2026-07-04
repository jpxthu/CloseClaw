//! Unit tests for IMPlugin trait default rendering and platform hook overrides.
//!
//! Tests cover:
//! - Default `render()` pipeline: Text → `parse_content_segments` → hooks
//! - Mock plugin verifying plain-text fallback path
//! - Edge cases: empty blocks, single/multi blocks, unclosed code fences,
//!   no language annotation
//!
//! Note: TerminalPlugin tests live in the main crate (src/im_adapter/plugin_tests.rs)
//! since TerminalPlugin is defined there.
//! Streaming tests live in streaming_tests.rs since the streaming renderer
//! is now tested directly via the StreamingRenderer trait.

#[cfg(test)]
mod tests {
    use crate::code_block::{parse_content_segments, ContentSegment};
    use crate::platforms::feishu::{FeishuAdapter, FeishuPlugin};
    use crate::plugin::{IMPlugin, RenderedOutput};
    use async_trait::async_trait;
    use closeclaw_common::processor::ContentBlock;
    use closeclaw_common::{AdapterError, NormalizedMessage};
    use std::sync::Arc;

    // =========================================================================
    // Mock plugin — uses only default trait methods (no overrides)
    // =========================================================================

    struct DefaultMockPlugin;

    #[async_trait]
    impl IMPlugin for DefaultMockPlugin {
        fn platform(&self) -> &str {
            "mock"
        }

        async fn parse_inbound(
            &self,
            _payload: &[u8],
        ) -> Result<Option<NormalizedMessage>, AdapterError> {
            Ok(None)
        }

        async fn send(
            &self,
            _output: &RenderedOutput,
            _peer_id: &str,
            _thread_id: Option<&str>,
        ) -> Result<(), AdapterError> {
            Ok(())
        }
    }

    // =========================================================================
    // Default render() pipeline tests
    // =========================================================================

    #[test]
    fn test_default_render_empty_blocks() {
        let plugin = DefaultMockPlugin;
        let output = plugin.render(&[], None);
        assert_eq!(output.msg_type, "text");
        assert_eq!(output.payload, serde_json::Value::String(String::new()));
    }

    #[test]
    fn test_default_render_single_text_block() {
        let plugin = DefaultMockPlugin;
        let blocks = vec![ContentBlock::Text("hello world".into())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "text");
        let text = output.payload.as_str().unwrap();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn test_default_render_code_block_with_language() {
        let plugin = DefaultMockPlugin;
        let text = "before\n```rust\nfn main() {}\n```\nafter";
        let blocks = vec![ContentBlock::Text(text.into())];
        let output = plugin.render(&blocks, None);
        let result = output.payload.as_str().unwrap();
        assert!(result.contains("before"));
        assert!(result.contains("```rust"));
        assert!(result.contains("fn main() {}"));
        assert!(result.contains("```"));
        assert!(result.contains("after"));
    }

    #[test]
    fn test_default_render_code_block_without_language() {
        let plugin = DefaultMockPlugin;
        let text = "```\nhello\n```";
        let blocks = vec![ContentBlock::Text(text.into())];
        let output = plugin.render(&blocks, None);
        let result = output.payload.as_str().unwrap();
        assert!(result.contains("```\nhello\n```"));
    }

    #[test]
    fn test_default_render_horizontal_rule() {
        let plugin = DefaultMockPlugin;
        let text = "above\n---\nbelow";
        let blocks = vec![ContentBlock::Text(text.into())];
        let output = plugin.render(&blocks, None);
        let result = output.payload.as_str().unwrap();
        assert!(result.contains("above"));
        assert!(result.contains("---"));
        assert!(result.contains("below"));
    }

    #[test]
    fn test_default_render_mixed_content() {
        let plugin = DefaultMockPlugin;
        let text = "intro\n\n```python\nprint('hi')\n```\n\n---\n\nconclusion";
        let blocks = vec![ContentBlock::Text(text.into())];
        let output = plugin.render(&blocks, None);
        let result = output.payload.as_str().unwrap();
        assert!(result.contains("intro"));
        assert!(result.contains("```python"));
        assert!(result.contains("print('hi')"));
        assert!(result.contains("---"));
        assert!(result.contains("conclusion"));
    }

    #[test]
    fn test_default_render_multiple_text_blocks() {
        let plugin = DefaultMockPlugin;
        let blocks = vec![
            ContentBlock::Text("first paragraph".into()),
            ContentBlock::Text("second paragraph".into()),
        ];
        let output = plugin.render(&blocks, None);
        let result = output.payload.as_str().unwrap();
        assert!(result.contains("first paragraph"));
        assert!(result.contains("second paragraph"));
    }

    #[test]
    fn test_default_render_non_text_blocks_ignored() {
        let plugin = DefaultMockPlugin;
        let blocks = vec![
            ContentBlock::Text("visible".into()),
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "exec".into(),
                input: "ls".into(),
            },
            ContentBlock::ToolResult {
                tool_call_id: "t1".into(),
                content: "output".into(),
            },
        ];
        let output = plugin.render(&blocks, None);
        let result = output.payload.as_str().unwrap();
        assert!(result.contains("visible"));
        assert!(!result.contains("exec"));
        assert!(!result.contains("output"));
    }

    #[test]
    fn test_default_render_unclosed_code_block() {
        let plugin = DefaultMockPlugin;
        let text = "```rust\nfn main() {}\nno close";
        let blocks = vec![ContentBlock::Text(text.into())];
        let output = plugin.render(&blocks, None);
        let result = output.payload.as_str().unwrap();
        assert!(result.contains("```rust"));
        assert!(result.contains("fn main() {}"));
        assert!(result.contains("no close"));
    }

    #[test]
    fn test_default_render_preserves_rendered_output_structure() {
        let plugin = DefaultMockPlugin;
        let blocks = vec![ContentBlock::Text("test".into())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "text");
        assert!(output.payload.is_string());
    }

    // =========================================================================
    // FeishuPlugin rendering tests
    // =========================================================================

    fn make_feishu_plugin() -> FeishuPlugin {
        let adapter = Arc::new(FeishuAdapter::new(
            "test_app".into(),
            "test_secret".into(),
            "test_token".into(),
        ));
        FeishuPlugin::new(adapter)
    }

    #[test]
    fn test_feishu_plugin_platform() {
        let plugin = make_feishu_plugin();
        assert_eq!(plugin.platform(), "feishu");
    }

    #[test]
    fn test_feishu_plugin_render_empty_blocks() {
        let plugin = make_feishu_plugin();
        let output = plugin.render(&[], None);
        assert_eq!(output.msg_type, "text");
    }

    #[test]
    fn test_feishu_plugin_render_simple_text() {
        let plugin = make_feishu_plugin();
        let blocks = vec![ContentBlock::Text("hello".into())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "text");
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert_eq!(text, "hello");
    }

    #[test]
    fn test_feishu_plugin_render_multiline_uses_card() {
        let plugin = make_feishu_plugin();
        let blocks = vec![ContentBlock::Text("line1\nline2\nline3".into())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "interactive");
        assert!(output.payload.is_object());
    }

    #[test]
    fn test_feishu_plugin_render_code_block_uses_card() {
        let plugin = make_feishu_plugin();
        let text = "```rust\nfn main() {}\n```";
        let blocks = vec![ContentBlock::Text(text.into())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "interactive");
    }

    #[test]
    fn test_feishu_plugin_render_mixed_content_uses_card() {
        let plugin = make_feishu_plugin();
        let text = "header\n```python\nprint('hi')\n```\n---\nfooter";
        let blocks = vec![ContentBlock::Text(text.into())];
        let output = plugin.render(&blocks, None);
        assert_eq!(output.msg_type, "interactive");
    }

    // =========================================================================
    // clean_content tests
    // =========================================================================

    #[test]
    fn test_default_clean_content_passthrough() {
        let plugin = DefaultMockPlugin;
        assert_eq!(plugin.clean_content("hello world"), "hello world");
    }

    #[test]
    fn test_default_clean_content_empty() {
        let plugin = DefaultMockPlugin;
        assert_eq!(plugin.clean_content(""), "");
    }

    // =========================================================================
    // init/shutdown default tests
    // =========================================================================

    #[tokio::test]
    async fn test_default_init_noop() {
        let plugin = DefaultMockPlugin;
        plugin.init().await.unwrap();
    }

    #[tokio::test]
    async fn test_default_shutdown_noop() {
        let plugin = DefaultMockPlugin;
        plugin.shutdown().await.unwrap();
    }

    // =========================================================================
    // ContentSegment integration tests
    // =========================================================================

    #[test]
    fn test_parse_segments_empty_string() {
        let segs = parse_content_segments("");
        assert!(segs.is_empty());
    }

    #[test]
    fn test_parse_segments_only_hr() {
        let segs = parse_content_segments("---");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], ContentSegment::Hr);
    }

    #[test]
    fn test_parse_segments_code_block_no_lang() {
        let segs = parse_content_segments("```\ncode\n```");
        assert_eq!(
            segs,
            vec![ContentSegment::CodeBlock {
                language: String::new(),
                code: "code".into(),
            }]
        );
    }

    #[test]
    fn test_parse_segments_code_block_with_lang() {
        let segs = parse_content_segments("```rust\nfn main() {}\n```");
        assert_eq!(
            segs,
            vec![ContentSegment::CodeBlock {
                language: "rust".into(),
                code: "fn main() {}".into(),
            }]
        );
    }

    #[test]
    fn test_parse_segments_unclosed_fence() {
        let segs = parse_content_segments("```rust\nfn main() {}");
        assert_eq!(
            segs,
            vec![
                ContentSegment::Markdown("```rust".into()),
                ContentSegment::Markdown("fn main() {}".into()),
            ]
        );
    }

    #[test]
    fn test_parse_segments_multiline_mixed() {
        let input = "text1\n\n```\ncode\n```\n\n---\n\ntext2";
        let segs = parse_content_segments(input);
        assert_eq!(
            segs,
            vec![
                ContentSegment::Markdown("text1".into()),
                ContentSegment::CodeBlock {
                    language: String::new(),
                    code: "code".into(),
                },
                ContentSegment::Hr,
                ContentSegment::Markdown("text2".into()),
            ]
        );
    }

    // =========================================================================
    // RenderedOutput serialization
    // =========================================================================

    #[test]
    fn test_rendered_output_serde_roundtrip() {
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": "hello"}}),
        };
        let json = serde_json::to_string(&output).unwrap();
        let deserialized: RenderedOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.msg_type, "text");
        assert_eq!(deserialized.payload, output.payload);
    }

    #[test]
    fn test_rendered_output_debug() {
        let output = RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::Value::String("test".into()),
        };
        let debug = format!("{:?}", output);
        assert!(debug.contains("RenderedOutput"));
        assert!(debug.contains("text"));
    }

    #[test]
    fn test_rendered_output_clone() {
        let output = RenderedOutput {
            msg_type: "interactive".into(),
            payload: serde_json::json!({"key": "value"}),
        };
        let cloned = output.clone();
        assert_eq!(cloned.msg_type, output.msg_type);
        assert_eq!(cloned.payload, output.payload);
    }

    // =====================================================================
    // shutdown_inbound / shutdown_outbound default delegation tests
    // =====================================================================

    /// Mock plugin that tracks shutdown calls to verify delegation.
    struct ShutdownTrackerPlugin {
        shutdown_called: std::sync::atomic::AtomicBool,
        inbound_called: std::sync::atomic::AtomicBool,
        outbound_called: std::sync::atomic::AtomicBool,
    }

    impl ShutdownTrackerPlugin {
        fn new() -> Self {
            Self {
                shutdown_called: std::sync::atomic::AtomicBool::new(false),
                inbound_called: std::sync::atomic::AtomicBool::new(false),
                outbound_called: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    #[async_trait]
    impl IMPlugin for ShutdownTrackerPlugin {
        fn platform(&self) -> &str {
            "tracker"
        }

        async fn parse_inbound(
            &self,
            _payload: &[u8],
        ) -> Result<Option<NormalizedMessage>, AdapterError> {
            Ok(None)
        }

        async fn send(
            &self,
            _output: &RenderedOutput,
            _peer_id: &str,
            _thread_id: Option<&str>,
        ) -> Result<(), AdapterError> {
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), AdapterError> {
            self.shutdown_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        async fn shutdown_inbound(&self) -> Result<(), AdapterError> {
            self.inbound_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            // Override to do something different from shutdown()
            Ok(())
        }

        async fn shutdown_outbound(&self) -> Result<(), AdapterError> {
            self.outbound_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            // Override to do something different from shutdown()
            Ok(())
        }
    }

    /// Default shutdown_inbound() delegates to shutdown().
    #[tokio::test]
    async fn test_default_shutdown_inbound_delegates_to_shutdown() {
        let plugin = DefaultMockPlugin;
        // DefaultMockPlugin uses trait defaults — shutdown_inbound calls shutdown
        let result = plugin.shutdown_inbound().await;
        assert!(result.is_ok());
    }

    /// Default shutdown_outbound() delegates to shutdown().
    #[tokio::test]
    async fn test_default_shutdown_outbound_delegates_to_shutdown() {
        let plugin = DefaultMockPlugin;
        let result = plugin.shutdown_outbound().await;
        assert!(result.is_ok());
    }

    /// Custom shutdown_inbound() does NOT call shutdown().
    #[tokio::test]
    async fn test_custom_shutdown_inbound_does_not_call_shutdown() {
        let plugin = ShutdownTrackerPlugin::new();
        plugin.shutdown_inbound().await.unwrap();
        assert!(
            plugin
                .inbound_called
                .load(std::sync::atomic::Ordering::SeqCst),
            "inbound_called should be true"
        );
        assert!(
            !plugin
                .shutdown_called
                .load(std::sync::atomic::Ordering::SeqCst),
            "shutdown should NOT be called when inbound is overridden"
        );
    }

    /// Custom shutdown_outbound() does NOT call shutdown().
    #[tokio::test]
    async fn test_custom_shutdown_outbound_does_not_call_shutdown() {
        let plugin = ShutdownTrackerPlugin::new();
        plugin.shutdown_outbound().await.unwrap();
        assert!(
            plugin
                .outbound_called
                .load(std::sync::atomic::Ordering::SeqCst),
            "outbound_called should be true"
        );
        assert!(
            !plugin
                .shutdown_called
                .load(std::sync::atomic::Ordering::SeqCst),
            "shutdown should NOT be called when outbound is overridden"
        );
    }

    /// DefaultMockPlugin shutdown_inbound and shutdown_outbound both succeed.
    #[tokio::test]
    async fn test_default_shutdown_inbound_outbound_both_ok() {
        let plugin = DefaultMockPlugin;
        assert!(plugin.shutdown_inbound().await.is_ok());
        assert!(plugin.shutdown_outbound().await.is_ok());
    }

    // =====================================================================
    // Auto-discovery mechanism tests
    // =====================================================================

    use crate::platforms::PlatformEntry;

    /// Verify that `inventory::iter::<PlatformEntry>` collects at least one
    /// entry with `name == "feishu"` — i.e. the feishu platform module was
    /// discovered at compile time via `inventory::submit!`.
    #[test]
    fn test_platform_entry_inventory_collects_feishu() {
        let found = inventory::iter::<PlatformEntry>().any(|entry| entry.name == "feishu");
        assert!(
            found,
            "inventory must contain at least one PlatformEntry with name \"feishu\""
        );
    }
}
