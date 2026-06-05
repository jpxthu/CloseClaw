// ── send_outbound / route_message / streaming thread_id tests ────────────

use crate::gateway::{Gateway, SessionManager};
use crate::im::{AdapterError, IMPlugin, NormalizedMessage};
use crate::llm::types::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent, UnifiedUsage};
use crate::processor_chain::DslParseResult;
use crate::renderer::RenderedOutput;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{PersistenceService, ReasoningLevel, SessionCheckpoint};
use async_trait::async_trait;
use futures::stream;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::{GatewayConfig, Message};

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        dm_scope: super::DmScope::PerAccountChannelPeer,
    }
}

fn make_message(to: &str, content: &str) -> Message {
    Message {
        id: "test_msg".to_string(),
        from: "user_1".to_string(),
        to: to.to_string(),
        content: content.to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    }
}

// ── Capturing plugin ─────────────────────────────────────────────────────

/// Mock plugin that captures thread_id from each `send` call.
struct CapturingPlugin {
    platform: String,
    captured_thread_id: std::sync::Mutex<Option<String>>,
}

impl CapturingPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            captured_thread_id: std::sync::Mutex::new(None),
        }
    }

    fn captured(&self) -> Option<String> {
        self.captured_thread_id.lock().unwrap().clone()
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
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: json!({"content": {"text": "response"}}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        *self.captured_thread_id.lock().unwrap() = thread_id.map(|s| s.to_string());
        Ok(())
    }
}

// ── Mock persistence ─────────────────────────────────────────────────────

struct MockPersistService {
    checkpoint: Mutex<Option<SessionCheckpoint>>,
}

#[async_trait]
impl PersistenceService for MockPersistService {
    async fn save_checkpoint(
        &self,
        _: &SessionCheckpoint,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, crate::session::persistence::PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn delete_checkpoint(
        &self,
        _: &str,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }
    async fn list_active_sessions(
        &self,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(Vec::new())
    }
    async fn restore_checkpoint(
        &self,
        _: &str,
    ) -> Result<Option<SessionCheckpoint>, crate::session::persistence::PersistenceError> {
        Ok(self.checkpoint.lock().await.clone())
    }
    async fn archive_checkpoint(
        &self,
        _: &SessionCheckpoint,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }
    async fn list_archived_sessions(
        &self,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(Vec::new())
    }
    async fn purge_checkpoint(
        &self,
        _: &str,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }
    async fn invalidate_session(
        &self,
        _: &str,
    ) -> Result<(), crate::session::persistence::PersistenceError> {
        Ok(())
    }
    async fn list_idle_sessions_for_agent(
        &self,
        _: &str,
        _: crate::session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(Vec::new())
    }
    async fn list_expired_archived_sessions_for_agent(
        &self,
        _: &str,
        _: crate::session::persistence::AgentRole,
        _: i64,
    ) -> Result<Vec<String>, crate::session::persistence::PersistenceError> {
        Ok(Vec::new())
    }
}

// ── Setup helper ─────────────────────────────────────────────────────────

async fn setup_with_thread_id(
    thread_id: Option<&str>,
) -> (Gateway, Arc<SessionManager>, Arc<CapturingPlugin>) {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let checkpoint = {
        let mut cp = SessionCheckpoint::new("mock:ou_sender:agent-1".to_string());
        cp.chat_id = Some("agent-1".to_string());
        cp.channel = Some("mock".to_string());
        if let Some(tid) = thread_id {
            cp.thread_id = Some(tid.to_string());
        }
        cp
    };
    let mock_storage = Arc::new(MockPersistService {
        checkpoint: Mutex::new(Some(checkpoint)),
    });
    let sm = Arc::new(SessionManager::new(
        &make_config(),
        Some(mock_storage),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(make_config(), Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    let msg = make_message("agent-1", "hello");
    let _sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    (gw, sm, plugin)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_send_outbound_forwards_thread_id() {
    let (gw, _sm, plugin) = setup_with_thread_id(Some("omt_from_ckpt")).await;
    let msg = make_message("agent-1", "hello");
    let sid = _sm.find_or_create("mock", &msg, None).await.unwrap();
    gw.send_outbound(&sid, "mock", "hello world", vec![])
        .await
        .unwrap();
    assert_eq!(
        plugin.captured().as_deref(),
        Some("omt_from_ckpt"),
        "plugin.send should receive thread_id from checkpoint"
    );
}

#[tokio::test]
async fn test_send_outbound_no_thread_id() {
    let (gw, _sm, plugin) = setup_with_thread_id(None).await;
    let msg = make_message("agent-1", "hello");
    let sid = _sm.find_or_create("mock", &msg, None).await.unwrap();
    gw.send_outbound(&sid, "mock", "hello world", vec![])
        .await
        .unwrap();
    assert!(
        plugin.captured().is_none(),
        "plugin.send should receive None when checkpoint \
         has no thread_id"
    );
}

#[tokio::test]
async fn test_route_message_forwards_thread_id() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let checkpoint = {
        let mut cp = SessionCheckpoint::new("mock:ou_sender:agent-1".to_string());
        cp.chat_id = Some("agent-1".to_string());
        cp.channel = Some("mock".to_string());
        cp.thread_id = Some("omt_route_tid".to_string());
        cp
    };
    let mock_storage = Arc::new(MockPersistService {
        checkpoint: Mutex::new(Some(checkpoint)),
    });
    let sm = Arc::new(SessionManager::new(
        &make_config(),
        Some(mock_storage),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(make_config(), Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    let setup_msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &setup_msg, None).await.unwrap();
    let mut msg = make_message("agent-1", "hello");
    msg.metadata.insert("session_id".into(), sid);
    gw.route_message("mock", msg, None).await.unwrap();
    assert_eq!(
        plugin.captured().as_deref(),
        Some("omt_route_tid"),
        "route_message should forward thread_id from checkpoint \
         to plugin.send"
    );
}

#[tokio::test]
async fn test_send_outbound_streaming_forwards_thread_id() {
    let plugin_for_stream: Arc<CapturingPlugin> = Arc::new(CapturingPlugin::new("mock"));
    let checkpoint = {
        let mut cp = SessionCheckpoint::new("mock:ou_sender:agent-1".to_string());
        cp.chat_id = Some("agent-1".to_string());
        cp.channel = Some("mock".to_string());
        cp.thread_id = Some("omt_stream_tid".to_string());
        cp
    };
    let mock_storage = Arc::new(MockPersistService {
        checkpoint: Mutex::new(Some(checkpoint)),
    });
    let sm = Arc::new(SessionManager::new(
        &make_config(),
        Some(mock_storage),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(make_config(), Arc::clone(&sm));
    gw.register_plugin(Arc::clone(&plugin_for_stream) as Arc<dyn IMPlugin>)
        .await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    let events = vec![
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
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(UnifiedUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: Some(0),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin_for_stream.clone();
    gw.send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();
    assert_eq!(
        plugin_for_stream.captured().as_deref(),
        Some("omt_stream_tid"),
        "send_outbound_streaming should forward thread_id to \
         plugin.send"
    );
}
