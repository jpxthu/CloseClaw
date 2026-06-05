//! Tests for Gateway::send_outbound — basic (non-renderer) tests (issue #469).

use async_trait::async_trait;
use closeclaw::gateway::{DmScope, Gateway, GatewayConfig, GatewayError, Message, SessionManager};
use closeclaw::im::{AdapterError, IMAdapter, IMPlugin, NormalizedMessage};
use closeclaw::llm::types::ContentBlock;
use closeclaw::processor_chain::dsl_parser::DslParseResult;
use closeclaw::processor_chain::{
    MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage,
};
use closeclaw::renderer::RenderedOutput;
use closeclaw::session::bootstrap::BootstrapMode;
use closeclaw::session::persistence::ReasoningLevel;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// MockOutboundProcessor — test processor for outbound chain.
#[derive(Debug)]
struct MockOutboundProcessor {
    name: String,
    output_content: String,
    output_suppress: bool,
}

#[async_trait]
impl MessageProcessor for MockOutboundProcessor {
    fn name(&self) -> &str {
        &self.name
    }
    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Outbound
    }
    fn priority(&self) -> u8 {
        10
    }
    async fn process(
        &self,
        _ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, closeclaw::processor_chain::error::ProcessError> {
        Ok(Some(ProcessedMessage {
            content: self.output_content.clone(),
            metadata: serde_json::Map::new(),
            suppress: self.output_suppress,
            content_blocks: vec![],
        }))
    }
}

/// TrackingPlugin — test plugin for the outbound chain.
///
/// Implements [`IMPlugin`]. Records `send()` invocations, the rendered
/// `msg_type` from the most recent `render()` call, the joined text content
/// of the rendered content blocks, and the dsl_result forwarded to
/// `render()`. `forced_msg_type` lets each test pin the rendered output to a
/// specific `msg_type` (`"text"` by default).
#[derive(Debug, Default)]
struct TrackingPlugin {
    /// Tracks whether `send()` has been called at least once.
    send_called: Mutex<bool>,
    /// Last `msg_type` returned by `render()`.
    last_msg_type: Mutex<Option<String>>,
    /// Last joined text content extracted from `render()`'s content blocks.
    last_content: Mutex<Option<String>>,
    /// Last `dsl_result` observed by `render()`.
    last_dsl_result: Mutex<Option<DslParseResult>>,
    /// Optional override for the rendered `msg_type`. Defaults to `"text"`.
    forced_msg_type: Option<String>,
}

impl TrackingPlugin {
    fn new() -> Self {
        Self::default()
    }

    fn with_msg_type(msg_type: &str) -> Self {
        Self {
            forced_msg_type: Some(msg_type.to_string()),
            ..Default::default()
        }
    }
}

#[async_trait]
impl IMPlugin for TrackingPlugin {
    fn platform(&self) -> &str {
        "tracking"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(Some(NormalizedMessage {
            platform: "tracking".to_string(),
            sender_id: "user_1".to_string(),
            peer_id: "chat_1".to_string(),
            content: "hi".to_string(),
            timestamp: 0,
            thread_id: None,
            account_id: None,
        }))
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let content: String = content_blocks
            .iter()
            .filter_map(|b| {
                if let ContentBlock::Text(s) = b {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        *self.last_content.lock().unwrap() = Some(content.clone());
        *self.last_dsl_result.lock().unwrap() = dsl_result.cloned();

        let msg_type = self
            .forced_msg_type
            .clone()
            .unwrap_or_else(|| "text".to_string());
        *self.last_msg_type.lock().unwrap() = Some(msg_type.clone());

        let payload = match msg_type.as_str() {
            "text" => serde_json::json!({"content": {"text": content}}),
            "interactive" => serde_json::json!({"card": {"elements": []}}),
            _ => serde_json::Value::Null,
        };
        RenderedOutput { msg_type, payload }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        *self.send_called.lock().unwrap() = true;
        Ok(())
    }
}

pub(crate) fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
    }
}

pub(crate) fn make_outbound_message(to: &str, content: &str) -> Message {
    Message {
        id: "msg_1".to_string(),
        from: "ou_sender".to_string(),
        to: to.to_string(),
        content: content.to_string(),
        channel: "tracking".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
    }
}

pub(crate) fn make_outbound_gw(config: GatewayConfig) -> (Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

#[tokio::test]
async fn test_send_outbound_no_registry_bypass() {
    let config = make_config();
    let (gw, sm) = make_outbound_gw(config);
    let plugin = Arc::new(TrackingPlugin::new());
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert!(*plugin.send_called.lock().unwrap());
    assert_eq!(
        plugin.last_msg_type.lock().unwrap().clone(),
        Some("text".to_string())
    );
}

#[tokio::test]
async fn test_send_outbound_text_path() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let registry = Arc::new({
        let mut r = closeclaw::processor_chain::ProcessorRegistry::new();
        r.register(Arc::new(MockOutboundProcessor {
            name: "text_processor".into(),
            output_content: "processed text".into(),
            output_suppress: false,
        }));
        r
    });
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    let plugin = Arc::new(TrackingPlugin::new());
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert!(*plugin.send_called.lock().unwrap());
    assert_eq!(
        plugin.last_msg_type.lock().unwrap().clone(),
        Some("text".to_string())
    );
}

#[tokio::test]
async fn test_send_outbound_interactive_path() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let registry = Arc::new({
        let mut r = closeclaw::processor_chain::ProcessorRegistry::new();
        r.register(Arc::new(MockOutboundProcessor {
            name: "card_processor".into(),
            output_content: r#"{"elements":[]}"#.into(),
            output_suppress: false,
        }));
        r
    });
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    let plugin = Arc::new(TrackingPlugin::with_msg_type("interactive"));
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert!(*plugin.send_called.lock().unwrap());
    assert_eq!(
        plugin.last_msg_type.lock().unwrap().clone(),
        Some("interactive".to_string())
    );
}

#[tokio::test]
async fn test_send_outbound_suppress() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let registry = Arc::new({
        let mut r = closeclaw::processor_chain::ProcessorRegistry::new();
        r.register(Arc::new(MockOutboundProcessor {
            name: "suppressor".into(),
            output_content: "ignored".into(),
            output_suppress: true,
        }));
        r
    });
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    let plugin = Arc::new(TrackingPlugin::new());
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    // Suppress short-circuits before render() and send() are called.
    assert!(!*plugin.send_called.lock().unwrap());
    assert!(plugin.last_content.lock().unwrap().is_none());
    assert!(plugin.last_msg_type.lock().unwrap().is_none());
}

#[tokio::test]
async fn test_send_outbound_unknown_session() {
    let (gw, _sm) = make_outbound_gw(make_config());
    let plugin = Arc::new(TrackingPlugin::new());
    gw.register_plugin(plugin.clone()).await;

    let result = gw
        .send_outbound("nonexistent-session", "tracking", "raw", vec![])
        .await;
    assert!(matches!(result, Err(GatewayError::MissingSessionId)));
    assert!(!*plugin.send_called.lock().unwrap());
}

#[tokio::test]
async fn test_send_outbound_unknown_channel() {
    let (gw, sm) = make_outbound_gw(make_config());
    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    let result = gw.send_outbound(&sid, "unknown", "raw", vec![]).await;
    assert!(matches!(result, Err(GatewayError::UnknownChannel(_))));
}

#[tokio::test]
async fn test_feishu_adapter_send_card_json_default() {
    // The IMAdapter trait still ships a default send_card_json impl that
    // returns UnsupportedOperation. This test pins that behavior so any
    // future change is caught.
    struct DummyAdapter;
    #[async_trait]
    impl IMAdapter for DummyAdapter {
        fn name(&self) -> &str {
            "dummy"
        }
        async fn handle_webhook(&self, _: &[u8]) -> Result<Message, AdapterError> {
            Err(AdapterError::InvalidPayload("x".into()))
        }
        async fn send_message(&self, _: &Message, _: Option<&str>) -> Result<(), AdapterError> {
            Err(AdapterError::InvalidPayload("x".into()))
        }
        async fn validate_signature(&self, _: &str, _: &[u8]) -> bool {
            true
        }
    }
    let result = DummyAdapter.send_card_json("chat_1", "{}", None).await;
    assert!(matches!(result, Err(AdapterError::UnsupportedOperation)));
}
