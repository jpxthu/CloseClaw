//! Tests for streaming error path: Thinking block preservation.
//!
//! Verifies that when a `StreamEvent::Error` occurs during streaming,
//! the `GatewayError::StreamError` returned includes all previously
//! accumulated content blocks (including complete Thinking blocks) in
//! its `partial_content` field.
//!
//! Per design doc §流式输出:
//! "不完整的响应不写入 message history；例外：错误发生前已完整生成的
//! Thinking 块（对应 BlockEnd 已到达）保留在对话历史中并参与后续上下文"

use crate::{GatewayConfig, Message, OutboundMeta, SessionManager};
use closeclaw_common::im_plugin::{
    AdapterError, NormalizedMessage, RenderedOutput, StreamingOutput,
};
use closeclaw_common::processor::{ContentBlock, DslParseResult, StreamEvent};
use closeclaw_common::{IMPlugin, StreamingRenderer};
use closeclaw_llm::types::{ContentBlockType, ContentDelta};
use closeclaw_session::persistence::{
    PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
};
use std::sync::Arc;

// ── Mock persistence ────────────────────────────────────────────────────────

struct MockPersist;

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
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

// ── Mock plugin ─────────────────────────────────────────────────────────────

struct NoopPlugin {
    platform: String,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl NoopPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }
}

#[async_trait::async_trait]
impl IMPlugin for NoopPlugin {
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
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": ""}}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
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

// ── Setup helper ───────────────────────────────────────────────────────────

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-streaming-error".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    }
}

async fn setup() -> (crate::Gateway, Arc<SessionManager>, String) {
    let config = test_config();
    let persist = Arc::new(MockPersist);
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(persist),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let plugin: Arc<dyn IMPlugin> = Arc::new(NoopPlugin::new("mock"));
    gw.register_plugin(plugin).await;
    let msg = Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: "agent-1".to_string(),
        content: "hello".to_string(),
        channel: "mock".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    (gw, sm, sid)
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// Normal path: a complete Thinking block is accumulated before Error.
///
/// Verifies that `partial_content` in the returned `StreamError` contains
/// the Thinking block that was fully generated (BlockEnd received) before
/// the error occurred.
#[tokio::test]
async fn streaming_error_preserves_thinking_block() {
    let (gw, _sm, sid) = setup().await;
    let plugin: Arc<dyn IMPlugin> = Arc::new(NoopPlugin::new("mock"));
    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "reasoning step 1".to_string(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: " and step 2".to_string(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::Error {
            message: "connection lost".to_string(),
        }),
    ];
    let stream = futures::stream::iter(events);
    let err = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin, OutboundMeta::default())
        .await
        .expect_err("should return StreamError on error event");
    match err {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "connection lost");
            assert_eq!(
                partial_content.len(),
                1,
                "partial_content should contain exactly 1 block"
            );
            assert_eq!(
                partial_content[0],
                ContentBlock::Thinking {
                    thinking: "reasoning step 1 and step 2".to_string(),
                    signature: None,
                },
                "partial_content should contain the complete Thinking block"
            );
        }
        other => panic!("expected StreamError, got: {other:?}"),
    }
}

/// Boundary: no blocks accumulated before Error.
///
/// Verifies that when an error occurs immediately (before any content),
/// `partial_content` is empty.
#[tokio::test]
async fn streaming_error_empty_when_no_blocks() {
    let (gw, _sm, sid) = setup().await;
    let plugin: Arc<dyn IMPlugin> = Arc::new(NoopPlugin::new("mock"));
    let events = vec![Ok::<_, String>(StreamEvent::Error {
        message: "early failure".to_string(),
    })];
    let stream = futures::stream::iter(events);
    let err = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin, OutboundMeta::default())
        .await
        .expect_err("should return StreamError");
    match err {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "early failure");
            assert!(
                partial_content.is_empty(),
                "partial_content should be empty when no blocks were accumulated"
            );
        }
        other => panic!("expected StreamError, got: {other:?}"),
    }
}

/// Multi-block: Text + Thinking blocks accumulated before Error.
///
/// Verifies that `partial_content` contains all previously accumulated
/// blocks in order.
#[tokio::test]
async fn streaming_error_preserves_multiple_blocks() {
    let (gw, _sm, sid) = setup().await;
    let plugin: Arc<dyn IMPlugin> = Arc::new(NoopPlugin::new("mock"));
    let events = vec![
        // Block 0: Text
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "hello".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: " world".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        // Block 1: Thinking
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Thinking {
                thinking: "internal reasoning".to_string(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Thinking,
        }),
        // Error
        Ok(StreamEvent::Error {
            message: "stream interrupted".to_string(),
        }),
    ];
    let stream = futures::stream::iter(events);
    let err = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin, OutboundMeta::default())
        .await
        .expect_err("should return StreamError");
    match err {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "stream interrupted");
            assert_eq!(
                partial_content.len(),
                2,
                "partial_content should contain both Text and Thinking blocks"
            );
            assert_eq!(
                partial_content[0],
                ContentBlock::Text("hello world".to_string()),
                "first block should be the accumulated Text block"
            );
            assert_eq!(
                partial_content[1],
                ContentBlock::Thinking {
                    thinking: "internal reasoning".to_string(),
                    signature: None,
                },
                "second block should be the accumulated Thinking block"
            );
        }
        other => panic!("expected StreamError, got: {other:?}"),
    }
}
