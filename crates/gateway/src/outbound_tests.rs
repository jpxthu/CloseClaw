//! Unit tests for verbosity filtering in outbound message routing.

use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::{ContentBlock, ContentBlockType};

use std::sync::Arc;

use super::outbound::filter_by_verbosity;

#[test]
fn test_filter_by_verbosity_full() {
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
    ];
    let result = filter_by_verbosity(blocks.clone(), VerbosityLevel::Full);
    assert_eq!(result.len(), 3);
    assert!(matches!(result[0], ContentBlock::Text(_)));
    assert!(matches!(result[1], ContentBlock::Thinking { .. }));
    assert!(matches!(result[2], ContentBlock::ToolUse { .. }));
}

#[test]
fn test_filter_by_verbosity_normal() {
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Normal);
    assert_eq!(result.len(), 2);
    assert!(matches!(result[0], ContentBlock::Text(_)));
    assert!(matches!(result[1], ContentBlock::ToolUse { .. }));
}

#[test]
fn test_filter_by_verbosity_off() {
    let blocks = vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "hello"));
}

#[test]
fn test_filter_empty_blocks() {
    let blocks = vec![];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Full);
    assert!(result.is_empty());

    let result = filter_by_verbosity(vec![], VerbosityLevel::Normal);
    assert!(result.is_empty());

    let result = filter_by_verbosity(vec![], VerbosityLevel::Off);
    assert!(result.is_empty());
}

// ---------------------------------------------------------------------------
// Streaming per-block verbosity filtering
// ---------------------------------------------------------------------------
// These tests verify that the per-block filtering logic used in
// `process_stream_event` (streaming path) produces the same results
// as `filter_by_verbosity` (batch path) for individual blocks.
// The streaming path checks each BlockEnd individually.

/// Simulate the per-block streaming filter logic from `process_stream_event`.
/// Returns `true` if the block should be filtered (hidden).
fn streaming_should_filter(block_type: &ContentBlockType, level: VerbosityLevel) -> bool {
    *block_type != ContentBlockType::Text
        && match level {
            VerbosityLevel::Normal => matches!(block_type, ContentBlockType::Thinking),
            VerbosityLevel::Off => true,
            VerbosityLevel::Full => false,
        }
}

#[test]
fn test_streaming_text_never_filtered() {
    for level in [
        VerbosityLevel::Full,
        VerbosityLevel::Normal,
        VerbosityLevel::Off,
    ] {
        assert!(!streaming_should_filter(&ContentBlockType::Text, level));
    }
}

#[test]
fn test_streaming_thinking_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Full
    ));
    assert!(streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Normal
    ));
    assert!(streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Off
    ));
}

#[test]
fn test_streaming_tool_use_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::ToolUse,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_tool_result_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::ToolResult,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_image_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::Image,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_audio_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::Audio,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_file_not_filtered_at_normal() {
    assert!(!streaming_should_filter(
        &ContentBlockType::File,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_streaming_non_text_filtered_at_off() {
    for bt in [
        ContentBlockType::Thinking,
        ContentBlockType::ToolUse,
        ContentBlockType::ToolResult,
        ContentBlockType::Image,
        ContentBlockType::Audio,
        ContentBlockType::File,
    ] {
        assert!(streaming_should_filter(&bt, VerbosityLevel::Off));
    }
}

#[test]
fn test_streaming_nothing_filtered_at_full() {
    for bt in [
        ContentBlockType::Text,
        ContentBlockType::Thinking,
        ContentBlockType::ToolUse,
        ContentBlockType::ToolResult,
        ContentBlockType::Image,
        ContentBlockType::Audio,
        ContentBlockType::File,
    ] {
        assert!(!streaming_should_filter(&bt, VerbosityLevel::Full));
    }
}

// ---------------------------------------------------------------------------
// Batch vs streaming consistency
// ---------------------------------------------------------------------------

#[test]
fn test_batch_normal_matches_streaming_for_thinking() {
    // Batch: Thinking blocks removed at Normal level.
    let blocks = vec![ContentBlock::Text("hi".into())];
    let batch_result = filter_by_verbosity(blocks, VerbosityLevel::Normal);
    assert_eq!(batch_result.len(), 1);
    // Streaming: Thinking block filtered at Normal level.
    assert!(streaming_should_filter(
        &ContentBlockType::Thinking,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_batch_normal_preserves_tool_result() {
    let blocks = vec![
        ContentBlock::Text("text".into()),
        ContentBlock::ToolResult {
            tool_call_id: "tc_1".into(),
            content: "result".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Normal);
    assert_eq!(
        result.len(),
        2,
        "ToolResult should not be filtered at Normal"
    );
    // Streaming: ToolResult not filtered at Normal.
    assert!(!streaming_should_filter(
        &ContentBlockType::ToolResult,
        VerbosityLevel::Normal
    ));
}

#[test]
fn test_batch_off_keeps_only_text() {
    let blocks = vec![
        ContentBlock::Text("keep".into()),
        ContentBlock::Thinking {
            thinking: "rm".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "t".into(),
            input: "{}".into(),
        },
        ContentBlock::Image {
            name: "img".into(),
            url: "https://example.com/img.png".into(),
        },
    ];
    let result = filter_by_verbosity(blocks, VerbosityLevel::Off);
    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], ContentBlock::Text(t) if t == "keep"));
}

// ===========================================================================
// Gateway-level three-step outbound flow tests
// ===========================================================================
// These tests verify that the outbound pipeline correctly executes:
//   Step 1: VerbosityFilter (in-chain, priority 5)
//   Step 2: DslParser (in-chain, priority 10)
//
// Since VerbosityFilter is now part of the processor chain, we test the
// combined flow by using a registry that includes both processors.

use closeclaw_common::processor::ProcessedMessage;

/// Build an outbound registry with VerbosityFilter + DslParser.
/// Mirrors the chain produced by `build_processor_registry` for default config.
fn build_full_outbound_chain() -> closeclaw_processor_chain::ProcessorRegistry {
    let mut registry = closeclaw_processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(
        closeclaw_processor_chain::verbosity_filter::VerbosityFilter,
    ));
    registry.register(Arc::new(closeclaw_processor_chain::DslParser));
    registry
}

/// Simulate the outbound pipeline: VerbosityFilter → DslParser chain.
fn simulate_outbound_pipeline(
    blocks: Vec<ContentBlock>,
    verbosity: VerbosityLevel,
) -> ProcessedMessage {
    let mut meta = std::collections::HashMap::new();
    meta.insert("verbosity_level".to_string(), verbosity.to_string());
    let input = ProcessedMessage {
        content_blocks: blocks,
        metadata: meta,
    };
    let registry = build_full_outbound_chain();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(registry.process_outbound(input)).unwrap()
}

#[test]
fn test_three_step_normal_filters_thinking_then_parses_dsl() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "internal reasoning".into(),
            signature: None,
        },
        ContentBlock::Text("::button[label:OK;action:submit]".into()),
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Normal);

    // VerbosityFilter (in-chain) filters Thinking blocks at Normal level
    // DslParser (in-chain) extracts DSL from remaining Text block
    // DslParser fallback keeps the original content when all blocks are stripped
    assert_eq!(
        result.content_blocks.len(),
        1,
        "DSL-only block: fallback keeps original text"
    );
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

#[test]
fn test_three_step_off_keeps_only_text_then_parses_dsl() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "rm".into(),
            signature: None,
        },
        ContentBlock::Text("Hello".into()),
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "t".into(),
            input: "{}".into(),
        },
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Off);

    // VerbosityFilter (in-chain) keeps only Text at Off level
    // DslParser (in-chain) processes Text
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello"));
}

#[test]
fn test_three_step_full_keeps_all_then_parses_dsl() {
    let blocks = vec![
        ContentBlock::Text("Hello".into()),
        ContentBlock::Thinking {
            thinking: "reasoning".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "t".into(),
            input: "{}".into(),
        },
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Full);

    // VerbosityFilter (in-chain) keeps all blocks at Full level
    // DslParser (in-chain) passes through non-Text blocks, Text passes unchanged
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello"));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::ToolUse { .. }
    ));
}

#[test]
fn test_three_step_empty_blocks() {
    let blocks: Vec<ContentBlock> = vec![];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Normal);

    // Empty blocks → DslParser fallback creates a Text block from content (empty)
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
}

#[test]
fn test_three_step_mixed_blocks_with_dsl() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "step 1".into(),
            signature: None,
        },
        ContentBlock::Text("Result here.".into()),
        ContentBlock::Text("::button[label:OK;action:ok]".into()),
        ContentBlock::ToolUse {
            id: "2".into(),
            name: "tool".into(),
            input: "{}".into(),
        },
    ];

    let result = simulate_outbound_pipeline(blocks, VerbosityLevel::Normal);

    // VerbosityFilter (in-chain) filters Thinking at Normal level
    // DslParser (in-chain): first Text kept, second Text (DSL-only) stripped, ToolUse passed
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Result here."));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::ToolUse { .. }
    ));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

// ===========================================================================
// Streaming error chunks preservation tests (Step 1.3)
// ===========================================================================

use crate::{DmScope, GatewayConfig, SessionManager};
use closeclaw_common::processor::{StreamEvent, UnifiedUsage};
use closeclaw_common::StreamingRenderer;
use closeclaw_session::persistence::ReasoningLevel;
use futures::stream;
use std::path::PathBuf;

/// Mock plugin that records thinking indicator calls for assertions.
struct ThinkingIndicatorMock {
    platform: String,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
    thinking_calls: Arc<std::sync::Mutex<Vec<bool>>>,
}

impl ThinkingIndicatorMock {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
            thinking_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl closeclaw_common::IMPlugin for ThinkingIndicatorMock {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> closeclaw_common::im_plugin::RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        closeclaw_common::im_plugin::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        _output: &closeclaw_common::im_plugin::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        Ok(())
    }

    fn send_thinking_indicator(&self, active: bool) {
        self.thinking_calls.lock().expect("lock").push(active);
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.renderer.lock().expect("lock").handle_event(event)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.renderer.lock().expect("lock").flush()
    }
}

fn streaming_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-streaming".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

/// Helper: set up Gateway for streaming tests with a session mapped to a chat.
async fn setup_streaming_gw(
    session_id: &str,
    plugin: Arc<dyn closeclaw_common::IMPlugin>,
) -> crate::Gateway {
    let config = streaming_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    // Map session → chat_id for send_outbound_streaming.
    // get_chat_id() returns agent_id from sessions map.
    sm.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: "chat_test".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(plugin).await;
    gw
}

/// Step 1.3 — StreamError carries partial_content blocks.
///
/// When the stream emits an error after receiving some content,
/// the error variant must include the already-accumulated content_blocks.
#[tokio::test]
async fn test_stream_error_preserves_partial_content() {
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = setup_streaming_gw("sess-err-1", Arc::clone(&plugin)).await;

    // Stream: text block start + partial text + error
    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "partial content".to_string(),
            },
        }),
        Ok(StreamEvent::Error {
            message: "stream interrupted".to_string(),
        }),
    ];
    let stream = stream::iter(events);

    let result = gw
        .send_outbound_streaming("sess-err-1", "mock", stream, &plugin)
        .await;

    match result {
        Err(crate::GatewayError::StreamError {
            message,
            partial_content,
        }) => {
            assert_eq!(message, "stream interrupted");
            assert_eq!(partial_content.len(), 1, "should have 1 partial block");
            assert!(
                matches!(&partial_content[0], ContentBlock::Text(t) if t == "partial content"),
                "partial block should be the text content"
            );
        }
        other => panic!("expected StreamError with partial_content, got {:?}", other),
    }
}

/// Step 1.3 — Error at stream start → empty partial_content, no panic.
#[tokio::test]
async fn test_stream_error_at_start_empty_content() {
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = setup_streaming_gw("sess-err-2", Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![Ok(StreamEvent::Error {
        message: "immediate failure".to_string(),
    })];
    let stream = stream::iter(events);

    let result = gw
        .send_outbound_streaming("sess-err-2", "mock", stream, &plugin)
        .await;

    match result {
        Err(crate::GatewayError::StreamError {
            partial_content, ..
        }) => {
            assert!(
                partial_content.is_empty(),
                "partial_content should be empty at stream start error"
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// Step 1.3 — Successful stream → no error, StreamResult returned.
#[tokio::test]
async fn test_stream_success_no_error() {
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = setup_streaming_gw("sess-ok", Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "complete".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(UnifiedUsage {
                prompt_tokens: 5,
                completion_tokens: 3,
                total_tokens: Some(8),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);

    let result = gw
        .send_outbound_streaming("sess-ok", "mock", stream, &plugin)
        .await;
    assert!(result.is_ok(), "successful stream should not error");
    let sr = result.unwrap();
    assert!(
        !sr.content_blocks.is_empty(),
        "content blocks should not be empty"
    );
}

/// Step 1.3 — Multiple blocks then error → all prior blocks preserved.
#[tokio::test]
async fn test_stream_error_preserves_multiple_blocks() {
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(ThinkingIndicatorMock::new("mock"));
    let gw = setup_streaming_gw("sess-err-3", Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "block1".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: closeclaw_common::ContentDelta::Text {
                text: "block2".to_string(),
            },
        }),
        Ok(StreamEvent::Error {
            message: "mid-stream error".to_string(),
        }),
    ];
    let stream = stream::iter(events);

    let result = gw
        .send_outbound_streaming("sess-err-3", "mock", stream, &plugin)
        .await;

    match result {
        Err(crate::GatewayError::StreamError {
            partial_content, ..
        }) => {
            assert_eq!(
                partial_content.len(),
                2,
                "should have 2 partial blocks before error"
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// Step 1.5 — Thinking BlockStart → send_thinking_indicator(true).
#[tokio::test]
async fn test_thinking_indicator_sends_on_block_start() {
    let mock = ThinkingIndicatorMock::new("mock");
    let calls_ref = mock.thinking_calls.clone();
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(mock);
    let gw = setup_streaming_gw("sess-think-1", Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Thinking {
                thinking: "reasoning".to_string(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: closeclaw_common::ContentDelta::Text {
                text: "answer".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: None,
        }),
    ];
    let stream = stream::iter(events);

    let _ = gw
        .send_outbound_streaming("sess-think-1", "mock", stream, &plugin)
        .await;

    let calls = calls_ref.lock().expect("lock").clone();
    // Should have [true, false] for BlockStart/BlockEnd pair
    assert_eq!(calls.len(), 2, "should have 2 indicator calls");
    assert!(calls[0], "first call should be true (BlockStart)");
    assert!(!calls[1], "second call should be false (BlockEnd)");
}

/// Step 1.5 — VerbosityLevel::Off suppresses thinking indicator.
#[tokio::test]
async fn test_thinking_indicator_suppressed_at_off() {
    use closeclaw_common::VerbosityLevel;

    let config = streaming_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let session_id = "sess-think-off";
    sm.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: "chat_off".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    // Set verbosity to Off on the ConversationSession.
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        cs_arc
            .write()
            .await
            .set_verbosity_level(VerbosityLevel::Off);
    }
    {
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert(session_id.to_string(), cs_arc);
    }

    let mock = ThinkingIndicatorMock::new("mock");
    let calls_ref = mock.thinking_calls.clone();
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(mock);
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Thinking {
                thinking: "reasoning".to_string(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: closeclaw_common::ContentDelta::Text {
                text: "answer".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: None,
        }),
    ];
    let stream = stream::iter(events);

    let _ = gw
        .send_outbound_streaming(session_id, "mock", stream, &plugin)
        .await;

    let calls = calls_ref.lock().expect("lock").clone();
    assert!(
        calls.is_empty(),
        "no thinking indicator calls expected at VerbosityLevel::Off"
    );
}

/// Step 1.5 — Thinking BlockEnd → send_thinking_indicator(false) (stop).
#[tokio::test]
async fn test_thinking_indicator_stops_on_block_end() {
    let mock = ThinkingIndicatorMock::new("mock");
    let calls_ref = mock.thinking_calls.clone();
    let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(mock);
    let gw = setup_streaming_gw("sess-think-end", Arc::clone(&plugin)).await;

    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Thinking {
                thinking: "thinking".to_string(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: None,
        }),
    ];
    let stream = stream::iter(events);

    let _ = gw
        .send_outbound_streaming("sess-think-end", "mock", stream, &plugin)
        .await;

    let calls = calls_ref.lock().expect("lock").clone();
    assert_eq!(calls, vec![true, false]);
}

/// Helper to provide default usage for tests that don't care about usage values.
fn default_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: Some(0),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}
