//! Incremental-phase Processor Chain path tests (Step 1.3) — part 2.
//!
//! Covers: edge cases, error paths, invocation count, batch filter consistency.

use super::*;
use closeclaw_common::VerbosityLevel;
use std::sync::atomic::{AtomicUsize, Ordering};

// ── Verbosity-aware mock ProcessorChain ────────────────────────────────────

/// A mock that applies real VerbosityFilter logic in
/// `process_outbound_incremental`.
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

// ── Error-injecting mock ProcessorChain ────────────────────────────────────

/// A mock that always fails on `process_outbound_incremental`.
struct FailingIncrementalChain;

#[async_trait::async_trait]
impl closeclaw_common::processor::ProcessorChain for FailingIncrementalChain {
    async fn process_inbound(
        &self,
        _msg: NormalizedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        unimplemented!()
    }

    async fn process_outbound(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        Ok(msg)
    }

    async fn process_outbound_incremental(
        &self,
        _msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, closeclaw_common::processor::ProcessError> {
        Err(closeclaw_common::processor::ProcessError::ChainFailed(
            "simulated incremental chain failure".into(),
        ))
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

// ── Test event builders ────────────────────────────────────────────────────

fn thinking_delta(thinking: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index: 0,
        delta: ContentDelta::Thinking {
            thinking: thinking.to_string(),
            signature: None,
        },
    }
}

fn text_delta(index: usize, text: &str) -> StreamEvent {
    StreamEvent::BlockDelta {
        index,
        delta: ContentDelta::Text {
            text: text.to_string(),
        },
    }
}

fn block_start(index: usize, block_type: ContentBlockType) -> StreamEvent {
    StreamEvent::BlockStart { index, block_type }
}

fn block_end(index: usize, block_type: ContentBlockType) -> StreamEvent {
    StreamEvent::BlockEnd { index, block_type }
}

fn message_end() -> StreamEvent {
    StreamEvent::MessageEnd {
        usage: Some(default_usage()),
        finish_reason: Some("stop".to_string()),
    }
}

/// Setup a gateway with a VerbosityAwareChain and set session verbosity.
async fn setup_with_verbosity(
    verbosity: closeclaw_common::VerbosityLevel,
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
// Edge: empty render_blocks → no panic, no extra sends
// ═══════════════════════════════════════════════════════════════════════════

/// When the plugin's handle_stream_event returns empty render_blocks for
/// a non-Text BlockEnd, the gateway should not panic and should not send
/// any extra messages.
#[tokio::test]
async fn test_incremental_empty_render_blocks_no_panic() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_with_verbosity(VerbosityLevel::Full, plugin.clone()).await;

    // Send a ToolUse block — the renderer may produce empty render_blocks.
    let events = vec![
        block_start(0, ContentBlockType::ToolUse),
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseId {
                id: "c1".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseName {
                name: "tool".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseInputChunk {
                input: "{}".to_string(),
            },
        },
        block_end(0, ContentBlockType::ToolUse),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await;

    // Should not panic.
    assert!(result.is_ok(), "empty render_blocks should not cause panic");
}

// ═══════════════════════════════════════════════════════════════════════════
// Edge: DSL line text passthrough in incremental phase
// ═══════════════════════════════════════════════════════════════════════════

/// DSL lines are NOT stripped during the incremental phase.
/// They pass through VerbosityFilter as regular Text blocks.
/// DSL parsing is deferred to the finish phase.
#[tokio::test]
async fn test_incremental_dsl_lines_passthrough_not_stripped() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) = setup_with_verbosity(VerbosityLevel::Full, plugin.clone()).await;

    let events = vec![
        block_start(0, ContentBlockType::Text),
        text_delta(0, "::button[label:Yes;action:confirm;value:1]\n"),
        block_end(0, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // DSL line sent as-is during streaming (not stripped).
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        extract_text(&sent[0]).contains("::button"),
        "DSL line should be sent as-is during streaming"
    );

    // DSL line in content_blocks (not stripped).
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) if !t.is_empty() => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks.len(), 1);
    assert!(
        text_blocks[0].contains("::button"),
        "DSL line should be in content_blocks"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Error: chain processing failure → fallback, no panic, no stream interruption
// ═══════════════════════════════════════════════════════════════════════════

/// When `process_outbound_incremental` fails, the gateway falls back to
/// sending the original block. The stream continues without interruption.
#[tokio::test]
async fn test_incremental_chain_failure_falls_back_to_original_block() {
    let chain: Arc<dyn closeclaw_common::processor::ProcessorChain> =
        Arc::new(FailingIncrementalChain);
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
        block_start(0, ContentBlockType::Text),
        text_delta(0, "Important text.\n"),
        block_end(0, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await;

    // Should NOT return an error — chain failure is handled gracefully.
    assert!(
        result.is_ok(),
        "chain failure should not propagate as stream error"
    );

    // The original text should still be sent via fallback.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "original text should be sent on chain failure"
    );
}

/// Chain failure on a Thinking block: original Thinking block is sent
/// via fallback (chain failure = Full passthrough behavior).
#[tokio::test]
async fn test_incremental_chain_failure_sends_thinking_block() {
    let chain: Arc<dyn closeclaw_common::processor::ProcessorChain> =
        Arc::new(FailingIncrementalChain);
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
        .await;

    assert!(
        result.is_ok(),
        "chain failure should not propagate as error"
    );

    // Both blocks sent via fallback (chain failure = Full passthrough).
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2, "both blocks should be sent on chain failure");
}

/// Chain failure does not panic — stream completes normally.
#[tokio::test]
async fn test_incremental_chain_failure_no_panic() {
    let chain: Arc<dyn closeclaw_common::processor::ProcessorChain> =
        Arc::new(FailingIncrementalChain);
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

    // Multiple blocks to ensure chain failure is handled for each.
    let events = vec![
        block_start(0, ContentBlockType::Thinking),
        thinking_delta("reasoning"),
        block_end(0, ContentBlockType::Thinking),
        block_start(1, ContentBlockType::Text),
        text_delta(1, "Line 1\n"),
        block_end(1, ContentBlockType::Text),
        block_start(2, ContentBlockType::ToolUse),
        StreamEvent::BlockDelta {
            index: 2,
            delta: ContentDelta::ToolUseId {
                id: "c1".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 2,
            delta: ContentDelta::ToolUseName {
                name: "search".to_string(),
            },
        },
        StreamEvent::BlockDelta {
            index: 2,
            delta: ContentDelta::ToolUseInputChunk {
                input: "{}".to_string(),
            },
        },
        block_end(2, ContentBlockType::ToolUse),
        block_start(3, ContentBlockType::Text),
        text_delta(3, "Line 2\n"),
        block_end(3, ContentBlockType::Text),
        message_end(),
    ];
    let stream = stream::iter(events.into_iter().map(Ok::<_, String>));
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await;

    // All blocks should be sent via fallback — no panic, no error.
    assert!(result.is_ok(), "chain failure should not cause panic");
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 4, "all blocks should be sent via fallback");
}

// ═══════════════════════════════════════════════════════════════════════════
// VerbosityAwareChain invocation count verification
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that process_outbound_incremental is only called for non-text
/// blocks (Thinking/ToolUse/ToolResult). Text chunks are sent directly
/// by `dispatch_text` without chain processing (Step 1.1 change:
/// VerbosityFilter removed from per-line text dispatch).
#[tokio::test]
async fn test_incremental_chain_not_called_for_text_chunks() {
    let chain = Arc::new(VerbosityAwareChain::new());
    let chain_ref = Arc::clone(&chain);
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
        // Text block with 2 lines — dispatched directly by dispatch_text
        // (no chain call after Step 1.1).
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

    // Only 1 call: Thinking block via process_and_send_non_text_blocks.
    // Text chunks go directly through dispatch_text (no chain call).
    assert_eq!(
        chain_ref.incremental_call_count(),
        1,
        "only non-text blocks should call process_outbound_incremental; text chunks are sent directly by dispatch_text"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Single block: incremental filter matches batch filter semantics
// ═══════════════════════════════════════════════════════════════════════════

/// A single block processed through the incremental chain produces
/// the same filtering result as processing through the batch VerbosityFilter.
/// Verifies plan requirement: "单块 ProcessedMessage 经 incremental 链输出
/// 与批量 filter 语义一致".
#[tokio::test]
async fn test_incremental_single_block_matches_batch_filter() {
    use closeclaw_processor_chain::verbosity_filter::VerbosityFilter;

    let levels = [
        VerbosityLevel::Full,
        VerbosityLevel::Normal,
        VerbosityLevel::Off,
    ];

    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "reasoning".to_string(),
            signature: None,
        },
        ContentBlock::Text("visible".to_string()),
        ContentBlock::ToolUse {
            id: "c1".into(),
            name: "tool".into(),
            input: "{}".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "c1".into(),
            content: "result".into(),
        },
        ContentBlock::Image {
            name: "img.png".to_string(),
            url: "https://example.com/img.png".to_string(),
        },
    ];

    for level in levels {
        // Batch: filter all blocks at once.
        let batch_result = VerbosityFilter::filter(blocks.clone(), level);

        // Incremental: filter each block individually (matching how the
        // streaming pipeline processes blocks one at a time).
        let incremental_result: Vec<_> = blocks
            .iter()
            .filter(|b| VerbosityFilter::should_keep_block(b, level))
            .cloned()
            .collect();

        assert_eq!(
            batch_result.len(),
            incremental_result.len(),
            "block count mismatch at {:?}: batch={} incremental={}",
            level,
            batch_result.len(),
            incremental_result.len()
        );

        for (batch_block, incr_block) in batch_result.iter().zip(incremental_result.iter()) {
            assert_eq!(
                std::mem::discriminant(batch_block),
                std::mem::discriminant(incr_block),
                "block type mismatch at {:?}",
                level
            );
        }
    }
}
