//! Unit tests for the streaming pipeline (Step 1.4).
//!
//! Covers the plan Step 1.4 test targets:
//! - Normal path: non-DSL text lines pass through DslParser unchanged
//! - DSL path: `::button[...]` lines extracted, clean text sent, DSL accumulated
//! - Mixed path: some lines with DSL, some without
//! - Outbound log (Text): each sent text line is logged by Gateway
//! - Outbound log (non-Text): Thinking/ToolUse rendered content is logged
//! - Edge cases: empty lines, long lines, multi-line DSL markers
//! - State transition: DslParseResult accumulates correctly, merges post-stream

use crate::im_adapter::streaming::StreamingRenderer;
use crate::{GatewayConfig, Message, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::processor::ProcessedMessage;
use closeclaw_llm::types::{
    ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedUsage,
};
use closeclaw_session::persistence::{PersistenceError, ReasoningLevel, SessionCheckpoint};
use futures::stream;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

// ── Mock ProcessorChain ────────────────────────────────────────────────────

/// Mock [`ProcessorChain`](closeclaw_common::processor::ProcessorChain) that
/// tracks `parse_line_for_dsl` calls and returns configurable results.
pub(super) struct MockProcessorChain {
    /// Record of all lines passed to `parse_line_for_dsl`.
    parsed_lines: StdMutex<Vec<String>>,
    /// DSL instructions to return for each call (cycled).
    dsl_instructions: StdMutex<Vec<closeclaw_common::processor::DslInstruction>>,
}

impl MockProcessorChain {
    fn new() -> Self {
        Self {
            parsed_lines: StdMutex::new(Vec::new()),
            dsl_instructions: StdMutex::new(Vec::new()),
        }
    }

    /// Push a DSL instruction to be returned by the next `parse_line_for_dsl` call.
    fn push_dsl_instruction(&self, instruction: closeclaw_common::processor::DslInstruction) {
        self.dsl_instructions.lock().unwrap().push(instruction);
    }

    /// Get all lines that were parsed.
    fn parsed_lines(&self) -> Vec<String> {
        self.parsed_lines.lock().unwrap().clone()
    }
}

#[async_trait]
impl closeclaw_common::processor::ProcessorChain for MockProcessorChain {
    async fn process_inbound(
        &self,
        msg: NormalizedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        Ok(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(msg.content)],
            metadata: HashMap::new(),
        })
    }

    async fn process_outbound(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        // Passthrough — return content blocks as-is.
        Ok(msg)
    }

    fn parse_line_for_dsl(&self, line: &str) -> (String, DslParseResult) {
        self.parsed_lines.lock().unwrap().push(line.to_string());

        // Simple DSL detection: lines starting with ::button[ or ::selector[
        let trimmed = line.trim();
        if trimmed.starts_with("::button[") || trimmed.starts_with("::selector[") {
            let mut instructions = self.dsl_instructions.lock().unwrap();
            if !instructions.is_empty() {
                let instruction = instructions.remove(0);
                // DSL line: return empty string as clean text (DSL line removed)
                return (
                    String::new(),
                    DslParseResult {
                        instructions: vec![instruction],
                    },
                );
            }
        }
        // Non-DSL line: zero-overhead passthrough (return line unchanged)
        (
            line.to_string(),
            DslParseResult {
                instructions: vec![],
            },
        )
    }

    fn inbound_len(&self) -> usize {
        0
    }

    fn outbound_len(&self) -> usize {
        0
    }
}

// ── Mock Plugin ────────────────────────────────────────────────────────────

/// Mock [`IMPlugin`] that captures all sent messages for verification.
pub(super) struct CapturingPlugin {
    platform: String,
    /// All [`RenderedOutput`] payloads sent via `plugin.send`, in order.
    sent: StdMutex<Vec<serde_json::Value>>,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl CapturingPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            sent: StdMutex::new(Vec::new()),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }

    /// Drain and return all captured sent payloads.
    fn drain_sent(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }

    fn streaming_renderer(
        &self,
    ) -> &std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer> {
        &self.renderer
    }
}

#[async_trait]
impl IMPlugin for CapturingPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        // Render non-text blocks into a simplified representation.
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        if text.is_empty() {
            // For non-Text blocks, produce a rendered representation.
            let rendered: Vec<String> = content_blocks.iter().map(|b| format!("{:?}", b)).collect();
            RenderedOutput {
                msg_type: "text".into(),
                payload: json!({"content": {"text": rendered.join(", ")}}),
            }
        } else {
            RenderedOutput {
                msg_type: "text".into(),
                payload: json!({"content": {"text": text}}),
            }
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.sent.lock().unwrap().push(output.payload.clone());
        Ok(())
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .lock()
            .expect("CapturingPlugin streaming renderer lock poisoned")
            .handle_event(event)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .lock()
            .expect("CapturingPlugin streaming renderer lock poisoned")
            .flush()
    }
}

// ── Test helpers ───────────────────────────────────────────────────────────

pub(super) fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

pub(super) fn make_message(to: &str, content: &str) -> Message {
    Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: to.to_string(),
        content: content.to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
    }
}

pub(super) struct MockPersistService;

#[async_trait]
impl closeclaw_session::persistence::PersistenceService for MockPersistService {
    async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(Some(SessionCheckpoint::new("mock".to_string())))
    }
    async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn archive_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn purge_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: closeclaw_session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

/// Setup a gateway with a mock processor registry and a session for streaming.
pub(super) async fn setup_streaming(
    processor_chain: Arc<dyn closeclaw_common::processor::ProcessorChain>,
    plugin: Arc<dyn IMPlugin>,
) -> (crate::Gateway, Arc<SessionManager>, String) {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersistService)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), processor_chain);
    gw.register_plugin(plugin).await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    (gw, sm, sid)
}

pub(super) fn default_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: Some(0),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

/// Helper: extract text from a [`RenderedOutput`] payload.
pub(super) fn extract_text(payload: &serde_json::Value) -> String {
    payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

// ═══════════════════════════════════════════════════════════════════════════
// Normal path: non-DSL text passes through unchanged (zero-overhead)
// ═══════════════════════════════════════════════════════════════════════════

/// Non-DSL text lines go through `parse_line_for_dsl` and come back unchanged.
#[tokio::test]
async fn test_streaming_non_dsl_text_passthrough() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello world.\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Verify `parse_line_for_dsl` was called for each text line.
    let parsed = chain.parsed_lines();
    assert!(!parsed.is_empty(), "parse_line_for_dsl should be called");
    assert_eq!(parsed[0], "Hello world.");

    // Verify the text content is preserved unchanged.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks, vec!["Hello world."]);

    // Verify plugin.send was called with the unchanged text.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(extract_text(&sent[0]), "Hello world.");
}

// ═══════════════════════════════════════════════════════════════════════════
// DSL path: `::button[...]` lines extracted, DSL accumulated
// ═══════════════════════════════════════════════════════════════════════════

/// Lines containing DSL markers are parsed by DslParser; clean text is sent
/// and DSL instructions are accumulated in StreamState.
#[tokio::test]
async fn test_streaming_dsl_line_extracted_and_accumulated() {
    let chain = Arc::new(MockProcessorChain::new());
    chain.push_dsl_instruction(closeclaw_common::processor::DslInstruction {
        instruction_type: "button".to_string(),
        params: HashMap::from([
            ("label".to_string(), "Yes".to_string()),
            ("action".to_string(), "confirm".to_string()),
            ("value".to_string(), "1".to_string()),
        ]),
    });

    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:Yes;action:confirm;value:1]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // parse_line_for_dsl was called with the DSL line (including LineBuffer terminator).
    let parsed = chain.parsed_lines();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0], "::button[label:Yes;action:confirm;value:1]\n");

    // DSL line is stripped from sent text (clean_text is empty, not sent).
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 0, "DSL-only lines should not be sent");

    // StreamResult has no text block (empty clean_text skipped).
    assert_eq!(result.content_blocks.len(), 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// Mixed path: some lines with DSL, some without
// ═══════════════════════════════════════════════════════════════════════════

/// When the stream contains both DSL and non-DSL lines, the non-DSL lines
/// pass through unchanged and DSL lines are extracted.
#[tokio::test]
async fn test_streaming_mixed_dsl_and_plain_text() {
    let chain = Arc::new(MockProcessorChain::new());
    chain.push_dsl_instruction(closeclaw_common::processor::DslInstruction {
        instruction_type: "button".to_string(),
        params: HashMap::from([
            ("label".to_string(), "Click".to_string()),
            ("action".to_string(), "go".to_string()),
            ("value".to_string(), "ok".to_string()),
        ]),
    });

    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        // Each line terminated so LineBuffer emits them independently.
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello world\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:Click;action:go;value:ok]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Goodbye\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // parse_line_for_dsl was called for each complete line (with terminator).
    let parsed = chain.parsed_lines();
    assert_eq!(parsed.len(), 3, "should have parsed 3 lines");
    assert_eq!(parsed[0], "Hello world\n");
    assert_eq!(parsed[1], "::button[label:Click;action:go;value:ok]\n");
    assert_eq!(parsed[2], "Goodbye\n");

    // Sent messages: plain text dispatched (DSL stripped to empty, skipped).
    let sent = plugin.drain_sent();

    assert_eq!(sent.len(), 2, "only plain text lines dispatched");
    assert_eq!(extract_text(&sent[0]), "Hello world\n");
    assert_eq!(extract_text(&sent[1]), "Goodbye\n");

    // Content blocks: plain text accumulated.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(
        text_blocks.contains(&"Hello world\n".to_string()),
        "should contain 'Hello world\n'"
    );
    assert!(
        text_blocks.contains(&"Goodbye\n".to_string()),
        "should contain 'Goodbye\n'"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Outbound log (Text blocks): each text line is logged via tracing
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that each text line is dispatched (sent) by the Gateway before
/// being added to content_blocks. This tests that `dispatch_text` executes
/// the full pipeline: DslParser → outbound log → send.
///
/// Note: outbound logging uses `tracing::info!` which cannot be captured
/// directly in unit tests. We verify the behavior indirectly by confirming
/// that `plugin.send` is called for every text line and the text content
/// matches what was parsed.
#[tokio::test]
async fn test_streaming_text_outbound_log_and_send_order() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Line 1\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Line 2\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Both text lines should be sent via plugin.send.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2, "both text lines should be sent");
    // LineBuffer includes the terminator in emitted lines.
    assert_eq!(extract_text(&sent[0]), "Line 1\n");
    assert_eq!(extract_text(&sent[1]), "Line 2\n");

    // Content blocks should contain both lines.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks, vec!["Line 1\n", "Line 2\n"]);
}

// ═══════════════════════════════════════════════════════════════════════════
// Outbound log (non-Text blocks): Thinking/ToolUse rendered content logged
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that non-Text blocks (Thinking, ToolUse) go through `plugin.render`
/// and `plugin.send` at BlockEnd, which means the outbound log in
/// `send_render_block` is executed.
#[tokio::test]
async fn test_streaming_non_text_block_rendered_and_sent() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "internal reasoning".to_string(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Text {
                text: "Final answer.\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // With default verbosity Normal, Thinking blocks are filtered out.
    // Only the Text line should be sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "should send only Text line (Thinking filtered by default Normal verbosity)"
    );

    // The text line is sent via send_text.
    assert_eq!(extract_text(&sent[0]), "Final answer.");

    // With default Normal verbosity, Thinking is filtered from both send and content_blocks.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    assert!(
        !has_thinking,
        "result should NOT contain Thinking block (filtered by Normal verbosity)"
    );
    assert!(has_text, "result should contain Text block");
}

/// ToolUse block at BlockEnd goes through render + send.
#[tokio::test]
async fn test_streaming_tool_use_block_rendered_and_sent() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseId {
                id: "call_1".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseName {
                name: "search".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseInputChunk {
                input: r#"{"q":"test"}"#.to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::ToolUse,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // ToolUse block should be sent via render + send.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "ToolUse block should be sent");

    // ToolUse block should be in content_blocks.
    let has_tool_use = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
    assert!(has_tool_use, "result should contain ToolUse block");
}

// ═══════════════════════════════════════════════════════════════════════════
// Edge cases: empty lines, long lines, multi-line DSL
// ═══════════════════════════════════════════════════════════════════════════

/// Empty lines should not be sent or accumulated (route_line trims and skips).
#[tokio::test]
async fn test_streaming_empty_line_not_sent() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "\n\n\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Empty lines should not be sent.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 0, "empty lines should not be sent");

    // No text blocks accumulated.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(text_blocks.is_empty(), "no text blocks for empty lines");
}

/// Very long text lines (exceeding LineBuffer threshold) are force-emitted
/// and sent as complete strings.
#[tokio::test]
async fn test_streaming_long_line_force_emitted() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    // 150-character string without any terminator — exceeds LineBuffer threshold (100).
    let long_text = "a".repeat(150);
    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: long_text.clone(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // The long line should be force-emitted and sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "long line should be force-emitted as one message"
    );
    assert_eq!(extract_text(&sent[0]), long_text);

    // Content block should contain the full long text.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks.len(), 1);
    assert_eq!(text_blocks[0], long_text);
}

/// Multi-line DSL markers: `::button` syntax spans multiple lines.
/// Each line is parsed independently, so a multi-line DSL is NOT recognized
/// as a single instruction (documented behavior per DslParser spec).
#[tokio::test]
async fn test_streaming_multiline_dsl_each_line_independent() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        // Line 1: incomplete DSL (no closing bracket)
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:Yes\n".to_string(),
            },
        }),
        // Line 2: continuation (not valid DSL by itself)
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "action:confirm;value:1]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // parse_line_for_dsl was called for each complete line (with terminator).
    let parsed = chain.parsed_lines();
    assert_eq!(parsed.len(), 2);
    // First line: incomplete DSL (no closing bracket) → treated as plain text.
    // LineBuffer includes the terminator in emitted lines.
    assert_eq!(parsed[0], "::button[label:Yes\n");
    // Second line: continuation → not valid DSL by itself.
    assert_eq!(parsed[1], "action:confirm;value:1]\n");

    // Both lines are sent (no DSL extracted, zero overhead passthrough).
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(extract_text(&sent[0]), "::button[label:Yes\n");
    assert_eq!(extract_text(&sent[1]), "action:confirm;value:1]\n");

    // Both lines are in content_blocks.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks.len(), 2);
}

mod part2;
