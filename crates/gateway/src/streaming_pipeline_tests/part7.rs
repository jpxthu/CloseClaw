//! Step 1.3 — VerbosityFilter block-boundary filtering verification tests.
//!
//! Verifies the plan's acceptance criteria after Step 1.1 (removal of
//! per-line VerbosityFilter from `dispatch_text`):
//!
//! 1. **Text blocks at all VerbosityLevel** are sent directly without
//!    VerbosityFilter processing.
//! 2. **Thinking blocks at BlockEnd** are filtered by VerbosityFilter
//!    (Full → keep, Normal/Off → remove).
//! 3. **ToolUse blocks at BlockEnd** are filtered by VerbosityFilter
//!    (Full/Normal → keep, Off → remove).
//! 4. **Mixed Text+Thinking event sequence**: Text lines dispatched
//!    line-by-line, Thinking filtered at BlockEnd.
//! 5. **Regression**: existing `dispatch_text` tests still pass.

use super::*;
use closeclaw_common::VerbosityLevel;
use std::sync::atomic::{AtomicUsize, Ordering};

// ── Verbosity-aware mock ProcessorChain (local copy) ────────────────────────

/// A mock that applies real VerbosityFilter logic in
/// `process_outbound_incremental`. Tracks invocation count.
struct VerbosityAwareChain {
    incremental_call_count: AtomicUsize,
}

impl VerbosityAwareChain {
    fn new() -> Self {
        Self {
            incremental_call_count: AtomicUsize::new(0),
        }
    }

    fn incremental_call_count(&self) -> usize {
        self.incremental_call_count.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl closeclaw_common::processor::ProcessorChain for VerbosityAwareChain {
    async fn process_inbound(
        &self,
        _msg: NormalizedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        unimplemented!("inbound not tested here")
    }

    async fn process_outbound(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        Ok(msg)
    }

    async fn process_outbound_incremental(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        self.incremental_call_count.fetch_add(1, Ordering::SeqCst);
        let level = msg
            .metadata
            .get("verbosity_level")
            .and_then(|v| v.parse().ok())
            .unwrap_or(VerbosityLevel::Full);
        let filtered = closeclaw_processor_chain::verbosity_filter::VerbosityFilter::filter(
            msg.content_blocks,
            level,
        );
        Ok(ProcessedMessage {
            content_blocks: filtered,
            metadata: msg.metadata,
        })
    }

    async fn process_outbound_without_verbosity(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        Ok(msg)
    }

    fn inbound_len(&self) -> usize {
        0
    }

    fn outbound_len(&self) -> usize {
        0
    }
}

// ── Local test event builders ──────────────────────────────────────────────

fn block_start(index: usize, block_type: ContentBlockType) -> StreamEvent {
    StreamEvent::BlockStart { index, block_type }
}

fn block_end(index: usize, block_type: ContentBlockType) -> StreamEvent {
    StreamEvent::BlockEnd { index, block_type }
}

fn text_delta(index: usize, text: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index,
        delta: ContentDelta::Text {
            text: text.to_string(),
        },
    }
}

fn thinking_delta(thinking: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: thinking.to_string(),
            signature: None,
        },
    }
}

fn message_end() -> StreamEvent {
    StreamEvent::MessageEnd {
        usage: Some(default_usage()),
        finish_reason: Some("stop".to_string()),
    }
}

/// Setup a gateway with a real VerbosityAwareChain and session at given
/// verbosity level. Returns (gateway, session_id).
async fn setup_real_chain(
    verbosity: VerbosityLevel,
    plugin: Arc<dyn IMPlugin>,
) -> (crate::Gateway, String) {
    let chain: Arc<dyn closeclaw_common::processor::ProcessorChain> =
        Arc::new(VerbosityAwareChain::new());
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersistService)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), chain);
    gw.register_plugin(plugin).await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    if let Some(cs) = sm.get_conversation_session(&sid).await {
        cs.write().await.set_verbosity_level(verbosity);
    }
    (gw, sid)
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Text blocks at all VerbosityLevel: sent directly, no VerbosityFilter
// ═══════════════════════════════════════════════════════════════════════════

/// At Full verbosity: Text lines are sent directly (no chain call).
#[tokio::test]
async fn test_text_direct_send_at_full_verbosity() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Full, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Text),
        text_delta(0, "Line A\n"),
        text_delta(0, "Line B\n"),
        block_end(0, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let _result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Both text lines sent directly via plugin.send.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2, "both text lines should be sent");
    assert_eq!(extract_text(&sent[0]), "Line A\n");
    assert_eq!(extract_text(&sent[1]), "Line B\n");
}

/// At Normal verbosity: Text lines are sent directly (no chain call).
#[tokio::test]
async fn test_text_direct_send_at_normal_verbosity() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Normal, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Text),
        text_delta(0, "Hello\n"),
        text_delta(0, "World\n"),
        block_end(0, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let _result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Both text lines sent directly.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2, "both text lines should be sent at Normal");
    assert_eq!(extract_text(&sent[0]), "Hello\n");
    assert_eq!(extract_text(&sent[1]), "World\n");
}

/// At Off verbosity: Text lines are still sent directly (Text always passes).
#[tokio::test]
async fn test_text_direct_send_at_off_verbosity() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Off, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Text),
        text_delta(0, "Always visible\n"),
        block_end(0, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let _result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Text sent directly even at Off verbosity.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "text should be sent at Off verbosity");
    assert_eq!(extract_text(&sent[0]), "Always visible\n");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Thinking blocks at BlockEnd: filtered by VerbosityFilter
// ═══════════════════════════════════════════════════════════════════════════

/// Full verbosity: Thinking block is sent at BlockEnd (not filtered).
#[tokio::test]
async fn test_thinking_block_end_full_verbosity_kept() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Full, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Thinking),
        thinking_delta("internal reasoning"),
        block_end(0, ContentBlockType::Thinking),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Answer.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Full: both Thinking and Text sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "Full verbosity: Thinking + Text should be sent"
    );

    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(has_thinking, "Thinking should be in result at Full level");
}

/// Normal verbosity: Thinking block is filtered at BlockEnd (not sent).
#[tokio::test]
async fn test_thinking_block_end_normal_verbosity_filtered() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Normal, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Thinking),
        thinking_delta("hidden reasoning"),
        block_end(0, ContentBlockType::Thinking),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Answer.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Normal: only Text sent (Thinking filtered at BlockEnd).
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "Normal verbosity: only Text should be sent (Thinking filtered)"
    );

    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(!has_thinking, "Thinking should be filtered at Normal level");
}

/// Off verbosity: Thinking block is filtered at BlockEnd (not sent).
#[tokio::test]
async fn test_thinking_block_end_off_verbosity_filtered() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Off, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Thinking),
        thinking_delta("hidden"),
        block_end(0, ContentBlockType::Thinking),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Visible.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Off: only Text sent (Thinking filtered).
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "Off verbosity: only Text should be sent");

    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(!has_thinking, "Thinking should be filtered at Off level");
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. ToolUse blocks at BlockEnd: filtered by VerbosityFilter
// ═══════════════════════════════════════════════════════════════════════════

/// Full verbosity: ToolUse block is sent at BlockEnd (not filtered).
#[tokio::test]
async fn test_tool_use_block_end_full_verbosity_kept() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Full, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::ToolUse),
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseId {
                id: "call_1".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseName {
                name: "search".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseInputChunk {
                input: r#"{"q":"test"}"#.to_string(),
            },
        },
        block_end(0, ContentBlockType::ToolUse),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Done.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Full: both ToolUse and Text sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "Full verbosity: ToolUse + Text should be sent"
    );

    let has_tool_use = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
    assert!(has_tool_use, "ToolUse should be in result at Full level");
}

/// Normal verbosity: ToolUse block is sent at BlockEnd (not filtered —
/// only Thinking is filtered at Normal level).
#[tokio::test]
async fn test_tool_use_block_end_normal_verbosity_kept() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Normal, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::ToolUse),
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseId {
                id: "call_1".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseName {
                name: "search".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseInputChunk {
                input: "{}".to_string(),
            },
        },
        block_end(0, ContentBlockType::ToolUse),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Result.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Normal: ToolUse + Text sent (only Thinking filtered at Normal).
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "Normal verbosity: ToolUse + Text should be sent"
    );

    let has_tool_use = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
    assert!(has_tool_use, "ToolUse should be in result at Normal level");
}

/// Off verbosity: ToolUse block is filtered at BlockEnd (not sent —
/// Off keeps only Text and media blocks).
#[tokio::test]
async fn test_tool_use_block_end_off_verbosity_filtered() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Off, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::ToolUse),
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseId {
                id: "call_1".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseName {
                name: "search".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseInputChunk {
                input: "{}".to_string(),
            },
        },
        block_end(0, ContentBlockType::ToolUse),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Only text.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Off: only Text sent (ToolUse filtered).
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "Off verbosity: only Text should be sent (ToolUse filtered)"
    );

    let has_tool_use = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
    assert!(!has_tool_use, "ToolUse should be filtered at Off level");

    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    assert!(has_text, "Text should be preserved at Off level");
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Mixed Text+Thinking: Text line-by-line, Thinking filtered at BlockEnd
// ═══════════════════════════════════════════════════════════════════════════

/// Full verbosity: Text lines dispatched line-by-line, Thinking block
/// sent at BlockEnd. Both types appear in sent messages.
#[tokio::test]
async fn test_mixed_text_thinking_full_sends_both() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Full, plugin.clone()).await;

    let events = vec![
        // Thinking block first.
        block_start(0, ContentBlockType::Thinking),
        thinking_delta("let me think..."),
        block_end(0, ContentBlockType::Thinking),
        // Text block with multiple lines.
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Line 1\n"),
        text_delta(1, "Line 2\n"),
        text_delta(1, "Line 3\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Full: Thinking (1) + 3 text lines = 4 sends.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        4,
        "Full: Thinking + 3 text lines should be sent"
    );
    assert_eq!(extract_text(&sent[1]), "Line 1\n");
    assert_eq!(extract_text(&sent[2]), "Line 2\n");
    assert_eq!(extract_text(&sent[3]), "Line 3\n");

    // Both types in result.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    let text_count = result
        .content_blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Text(_)))
        .count();
    assert!(has_thinking, "Thinking should be in result at Full level");
    assert_eq!(text_count, 3, "3 text lines should be in result");
}

/// Normal verbosity: Text lines dispatched line-by-line, Thinking block
/// filtered at BlockEnd (not sent). Only text lines appear in sent messages.
#[tokio::test]
async fn test_mixed_text_thinking_normal_filters_thinking() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Normal, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Thinking),
        thinking_delta("hidden reasoning"),
        block_end(0, ContentBlockType::Thinking),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "First.\n"),
        text_delta(1, "Second.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Normal: Thinking filtered, only text lines sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "Normal: only 2 text lines should be sent (Thinking filtered)"
    );
    assert_eq!(extract_text(&sent[0]), "First.");
    assert_eq!(extract_text(&sent[1]), "Second.");

    // No Thinking in result.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(!has_thinking, "Thinking should be filtered at Normal level");
}

/// Off verbosity: Text lines dispatched line-by-line, Thinking block
/// filtered at BlockEnd. Only text lines appear in sent messages.
#[tokio::test]
async fn test_mixed_text_thinking_off_filters_thinking() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Off, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Thinking),
        thinking_delta("hidden"),
        block_end(0, ContentBlockType::Thinking),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Only this.\n"),
        block_end(1, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Off: only text sent.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "Off: only text line should be sent");
    assert_eq!(extract_text(&sent[0]), "Only this.");

    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(!has_thinking, "Thinking should be filtered at Off level");
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Regression: dispatch_text sends all text lines
// ═══════════════════════════════════════════════════════════════════════════

/// Regression: multiple text lines in a single Text block are all sent
/// via `dispatch_text` without any VerbosityFilter processing.
#[tokio::test]
async fn test_regression_dispatch_text_sends_all_lines() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Off, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Text),
        text_delta(0, "First line\n"),
        text_delta(0, "Second line\n"),
        text_delta(0, "Third line\n"),
        block_end(0, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // All 3 text lines sent (even at Off verbosity — Text always passes).
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        3,
        "all text lines should be sent at Off verbosity"
    );
    assert_eq!(extract_text(&sent[0]), "First line\n");
    assert_eq!(extract_text(&sent[1]), "Second line\n");
    assert_eq!(extract_text(&sent[2]), "Third line\n");

    // All 3 text lines in content_blocks.
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
        3,
        "all text lines should be in content_blocks"
    );
}

/// Regression: partial text at BlockEnd is also sent directly by
/// `dispatch_text` (flush path), no VerbosityFilter applied.
#[tokio::test]
async fn test_regression_dispatch_text_flush_sends_directly() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_real_chain(VerbosityLevel::Off, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Text),
        text_delta(0, "partial without newline"),
        block_end(0, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Flush path: partial text sent directly.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "flush should send the partial text");
    assert_eq!(extract_text(&sent[0]), "partial without newline");

    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks.len(), 1);
    assert_eq!(text_blocks[0], "partial without newline");
}

// ═══════════════════════════════════════════════════════════════════════════
// Chain invocation count: text chunks do NOT call process_outbound_incremental
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that process_outbound_incremental is NOT called for text chunks.
/// Only non-text blocks (Thinking/ToolUse) go through the chain at BlockEnd.
#[tokio::test]
async fn test_text_chunks_skipped_by_incremental_chain() {
    let chain = Arc::new(VerbosityAwareChain::new());
    let chain_ref = chain.clone();
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersistService)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), chain);
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    gw.register_plugin(plugin.clone()).await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();

    let events = vec![
        // Text block with 2 lines — dispatched directly by dispatch_text.
        block_start(0, ContentBlockType::Text),
        text_delta(0, "Line 1\n"),
        text_delta(0, "Line 2\n"),
        block_end(0, ContentBlockType::Text),
        // Thinking block — processed through chain at BlockEnd.
        block_start(1, ContentBlockType::Thinking),
        thinking_delta("reasoning"),
        block_end(1, ContentBlockType::Thinking),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    gw.send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Only 1 call: Thinking block. Text chunks go directly through dispatch_text.
    assert_eq!(
        chain_ref.incremental_call_count(),
        1,
        "text chunks should NOT call process_outbound_incremental; \
         only non-text blocks should trigger chain processing"
    );
}
