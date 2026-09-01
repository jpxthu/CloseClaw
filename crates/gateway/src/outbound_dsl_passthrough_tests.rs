//! Tests for DslParser pass-through in the streaming incremental phase.
//!
//! Verifies three behaviors specified by the design doc:
//! 1. Text block with DSL → parse succeeds, metadata contains dsl_result.
//! 2. Text block without DSL → pass-through, content unchanged.
//! 3. DslParser exception → fallback to original Text block.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{
    AdapterError, IMPlugin, NormalizedMessage, RenderedOutput, StreamingOutput,
};
use closeclaw_common::processor::{DslParseResult, StreamEvent};
use closeclaw_common::{ContentBlock, ContentBlockType, StreamingRenderer};
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};

use crate::{GatewayConfig, Message, OutboundMeta, SessionManager};

// ---------------------------------------------------------------------------
// Mock persistence
// ---------------------------------------------------------------------------

struct PassthroughMockPersist;

#[async_trait]
impl PersistenceService for PassthroughMockPersist {
    async fn save_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _a: &str,
        _r: closeclaw_session::persistence::AgentRole,
        _m: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _a: &str,
        _r: closeclaw_session::persistence::AgentRole,
        _m: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Mock plugin
// ---------------------------------------------------------------------------

struct CapturingPlugin {
    sent: tokio::sync::Mutex<Vec<serde_json::Value>>,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl CapturingPlugin {
    fn new() -> Self {
        Self {
            sent: tokio::sync::Mutex::new(Vec::new()),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }
}

#[async_trait]
impl IMPlugin for CapturingPlugin {
    fn platform(&self) -> &str {
        "mock"
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
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }
    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.sent.lock().await.push(output.payload.clone());
        Ok(())
    }
    fn send_thinking_indicator(&self, _active: bool) {}
    fn handle_stream_event(&self, event: StreamEvent) -> StreamingOutput {
        self.renderer.lock().expect("lock").handle_event(event)
    }
    fn flush_stream(&self) -> StreamingOutput {
        self.renderer.lock().expect("lock").flush()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-dsl-passthrough".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

fn default_usage() -> closeclaw_llm::types::UnifiedUsage {
    closeclaw_llm::types::UnifiedUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: Some(0),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

fn make_plugin() -> Arc<dyn IMPlugin> {
    Arc::new(CapturingPlugin::new())
}

async fn setup_gateway(plugin: Arc<dyn IMPlugin>) -> (crate::Gateway, Arc<SessionManager>, String) {
    let config = test_config();
    let persist: Arc<dyn PersistenceService> = Arc::new(PassthroughMockPersist);
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::clone(&persist)),
        None,
        ReasoningLevel::default(),
    ));
    let mut registry = closeclaw_processor_chain::registry::ProcessorRegistry::new();
    registry.register(Arc::new(closeclaw_processor_chain::DslParser));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));
    gw.register_plugin(plugin).await;
    let msg = Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: "agent-1".to_string(),
        content: "hello".to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    (gw, sm, sid)
}

fn text_events(text: &str) -> Vec<Result<StreamEvent, String>> {
    vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: format!("{}\n", text),
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
    ]
}

fn dsl_events() -> Vec<Result<StreamEvent, String>> {
    vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "Please confirm:\n::button[label:Yes;action:confirm;value:1]".to_string(),
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
    ]
}

fn empty_dsl_events() -> Vec<Result<StreamEvent, String>> {
    // Malformed DSL: ::button[label:X] missing required 'action' param.
    // DslParser should not parse it, resulting in empty instructions.
    vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "::button[label:X]\n".to_string(),
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
    ]
}

// ===========================================================================
// Tests
// ===========================================================================

/// Text block with DSL → parse succeeds, StreamResult contains dsl_result
/// with instructions and the content is preserved.
#[tokio::test]
async fn test_dsl_text_block_parses_and_preserves_content() {
    let plugin = make_plugin();
    let (gw, _sm, sid) = setup_gateway(plugin).await;

    let events = dsl_events();
    let stream = futures::stream::iter(events);
    let result = gw
        .send_outbound_streaming(
            &sid,
            "mock",
            stream,
            &make_plugin(),
            OutboundMeta::default(),
        )
        .await;
    let sr = result.expect("streaming should succeed");

    // dsl_result should contain the parsed instruction.
    assert!(sr.dsl_result.is_some(), "dsl_result should be present");
    let dsl: DslParseResult = serde_json::from_str(sr.dsl_result.as_ref().unwrap()).unwrap();
    assert_eq!(dsl.instructions.len(), 1);
    assert_eq!(dsl.instructions[0].instruction_type, "button");
    assert_eq!(dsl.instructions[0].params["label"], "Yes");

    // Content blocks should preserve the original text.
    let text = sr
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        text.contains("Please confirm:"),
        "content should contain the original text"
    );
}

/// Text block without DSL → pass-through, content unchanged, dsl_result
/// is None (no DslParser instructions found).
#[tokio::test]
async fn test_no_dsl_text_block_passthrough() {
    let plugin = make_plugin();
    let (gw, _sm, sid) = setup_gateway(plugin).await;

    let events = text_events("Hello, no DSL here!");
    let stream = futures::stream::iter(events);
    let result = gw
        .send_outbound_streaming(
            &sid,
            "mock",
            stream,
            &make_plugin(),
            OutboundMeta::default(),
        )
        .await;
    let sr = result.expect("streaming should succeed");

    // No DSL instructions → dsl_result should be Some with empty instructions.
    // DslParser always inserts its result into metadata.
    assert!(
        sr.dsl_result.is_some(),
        "dsl_result should be Some when DslParser runs (even with no DSL)"
    );
    let dsl: DslParseResult = serde_json::from_str(sr.dsl_result.as_ref().unwrap()).unwrap();
    assert!(
        dsl.instructions.is_empty(),
        "dsl_result instructions should be empty when no DSL present"
    );

    // Content should be the original text.
    let text = sr
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        text.contains("Hello, no DSL here!"),
        "content should contain original text, got: {}",
        text
    );
}

/// DslParser "exception" (malformed DSL not parseable) → fallback to original
/// Text block. Instructions are empty, content is unchanged.
#[tokio::test]
async fn test_malformed_dsl_fallback_to_original_text() {
    let plugin = make_plugin();
    let (gw, _sm, sid) = setup_gateway(plugin).await;

    // ::button[label:X] is missing the required 'action' param.
    // DslParser will not parse it → instructions empty → fallback to original.
    let events = empty_dsl_events();
    let stream = futures::stream::iter(events);
    let result = gw
        .send_outbound_streaming(
            &sid,
            "mock",
            stream,
            &make_plugin(),
            OutboundMeta::default(),
        )
        .await;
    let sr = result.expect("streaming should succeed");

    // No instructions parsed → dsl_result should be Some with empty instructions.
    assert!(
        sr.dsl_result.is_some(),
        "dsl_result should be Some for malformed DSL"
    );
    let dsl: DslParseResult = serde_json::from_str(sr.dsl_result.as_ref().unwrap()).unwrap();
    assert!(
        dsl.instructions.is_empty(),
        "dsl_result instructions should be empty for malformed DSL"
    );

    // Original text preserved (passthrough).
    let text = sr
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        text.contains("::button[label:X]"),
        "original malformed DSL text should be preserved, got: {}",
        text
    );
}

/// Multiple Text blocks: first has DSL, second does not.
/// Verifies incremental DSL extraction across block boundaries.
/// Note: the batch DslParser in finish_streaming_pipeline strips DSL
/// lines from content_blocks, so DSL text is not in the final output.
/// This test verifies (1) DSL instructions are extracted from Block 0
/// and (2) non-DSL text from Block 1 is preserved in content_blocks.
#[tokio::test]
async fn test_multiple_text_blocks_incremental_accumulation() {
    let plugin = make_plugin();
    let (gw, _sm, sid) = setup_gateway(plugin).await;

    let events: Vec<Result<StreamEvent, String>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "::button[label:Yes;action:confirm;value:1]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: closeclaw_common::ContentDelta::Text {
                text: "No DSL in second block\n".to_string(),
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

    let stream = futures::stream::iter(events);
    let result = gw
        .send_outbound_streaming(
            &sid,
            "mock",
            stream,
            &make_plugin(),
            OutboundMeta::default(),
        )
        .await;
    let sr = result.expect("streaming should succeed");

    // Should have exactly 1 instruction extracted from the first block.
    assert!(sr.dsl_result.is_some());
    let dsl: DslParseResult = serde_json::from_str(sr.dsl_result.as_ref().unwrap()).unwrap();
    assert_eq!(dsl.instructions.len(), 1);
    assert_eq!(dsl.instructions[0].instruction_type, "button");

    // Non-DSL text from Block 1 should be preserved in content_blocks.
    let all_text = sr
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        all_text.contains("No DSL in second block"),
        "non-DSL text should be preserved, got: {}",
        all_text
    );
}
