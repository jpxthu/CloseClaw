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
use crate::{GatewayConfig, Message, OutboundMeta, SessionManager};
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
                // DSL line: return empty string as clean text (DSL stripped)
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
        platform: None,
        dsl_result: None,
        content_blocks: None,
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

/// Non-DSL text lines pass through unchanged — no DSL parsing in
/// incremental streaming phase (DslParser is deferred to post-stream).
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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // parse_line_for_dsl is called for each text chunk during streaming.
    let parsed = chain.parsed_lines();
    assert_eq!(
        parsed,
        vec!["Hello world."],
        "parse_line_for_dsl should be called for each text chunk"
    );

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

/// DSL lines are sent as-is during streaming — no DslParser in incremental
/// phase. DSL parsing is deferred to the post-stream Processor Chain.
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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // parse_line_for_dsl is called for each text chunk during streaming.
    let parsed = chain.parsed_lines();
    assert_eq!(
        parsed,
        vec!["::button[label:Yes;action:confirm;value:1]\n"],
        "parse_line_for_dsl should be called for each text chunk"
    );

    // DSL line is stripped — only clean text (empty for pure DSL) is sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        0,
        "DSL line should be stripped, no clean text to send"
    );

    // StreamResult: DSL line is stripped, no non-empty text block for pure DSL.
    // Note: make_outbound_input may produce a Text("") fallback block.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) if !t.is_empty() => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(
        text_blocks.is_empty(),
        "DSL line should be stripped from content_blocks"
    );
    // DSL instruction should be accumulated in result.
    let dsl = result
        .dsl_result
        .as_ref()
        .map(|s| serde_json::from_str::<closeclaw_common::processor::DslParseResult>(s).unwrap());
    assert!(dsl.is_some(), "dsl_result should be present");
    assert_eq!(dsl.unwrap().instructions.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// Mixed path: some lines with DSL, some without
// ═══════════════════════════════════════════════════════════════════════════

/// When the stream contains both DSL and non-DSL lines, all lines are
/// sent as-is during streaming — no DSL parsing in incremental phase.
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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // parse_line_for_dsl is called for each text chunk during streaming.
    let parsed = chain.parsed_lines();
    assert_eq!(
        parsed.len(),
        3,
        "parse_line_for_dsl should be called for each text chunk"
    );

    // DSL line is stripped — only clean text lines are sent.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2, "only non-DSL lines should be dispatched");
    assert_eq!(extract_text(&sent[0]), "Hello world\n");
    assert_eq!(extract_text(&sent[1]), "Goodbye\n");

    // Content blocks: only clean text lines (DSL stripped).
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        text_blocks.len(),
        2,
        "DSL line should be stripped from content_blocks"
    );
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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
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
    let (gw, sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    // Explicitly set verbosity to Full so this test validates non-Text
    // block rendering behavior at Full verbosity (not affected by default).
    if let Some(cs) = sm.get_conversation_session(&sid).await {
        cs.write()
            .await
            .set_verbosity_level(closeclaw_common::VerbosityLevel::Full);
    }

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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Step 1.6: In Full mode (set explicitly), both Thinking and Text blocks are
    // sent via send_render_block during streaming.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "both Thinking and Text should be sent during streaming in Full mode"
    );

    // Mock is passthrough, so Thinking block is still in content_blocks
    // (pushed during streaming) and NOT filtered by the mock processor chain.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    assert!(
        has_thinking,
        "Thinking block should be in content_blocks (passthrough chain)"
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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Empty lines should not be sent.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 0, "empty lines should not be sent");

    // Empty lines should not be sent or accumulated in the incremental phase.
    // Note: finish_streaming_pipeline may produce a Text("") block via
    // make_outbound_input fallback when content_blocks is empty.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) if !t.is_empty() => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(
        text_blocks.is_empty(),
        "no non-empty text blocks for empty lines"
    );
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
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
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

mod part2;
mod part3;
mod part4;
