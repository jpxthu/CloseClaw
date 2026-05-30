//! Tests for Gateway::send_outbound — renderer integration tests (issue #469).

mod gateway_send_outbound_basic;

use async_trait::async_trait;
use closeclaw::gateway::{Gateway, GatewayError, Message, SessionManager};
use closeclaw::im::IMAdapter;
use closeclaw::processor_chain::{
    dsl_parser::DslParseResult, MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage,
};
use closeclaw::renderer::{RenderedOutput, Renderer};
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
    #[allow(dead_code)]
    dsl_result_json: Option<String>,
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
        let mut metadata = serde_json::Map::new();
        if let Some(ref json) = self.dsl_result_json {
            metadata.insert(
                "dsl_result".to_string(),
                serde_json::Value::String(json.clone()),
            );
        }
        Ok(Some(ProcessedMessage {
            content: self.output_content.clone(),
            metadata,
            suppress: self.output_suppress,
            content_blocks: vec![],
        }))
    }
}

/// MockRenderer — records render calls and returns configurable output.
#[derive(Debug)]
struct MockRenderer {
    platform_name: String,
    render_content: Mutex<Option<String>>,
    render_dsl_result: Mutex<Option<DslParseResult>>,
    output: RenderedOutput,
}

impl MockRenderer {
    fn new(msg_type: &str, payload: serde_json::Value) -> Self {
        Self {
            platform_name: "mock".to_string(),
            render_content: Mutex::new(None),
            render_dsl_result: Mutex::new(None),
            output: RenderedOutput {
                msg_type: msg_type.to_string(),
                payload,
            },
        }
    }
}

impl Renderer for MockRenderer {
    fn platform(&self) -> &str {
        &self.platform_name
    }
    fn render(&self, content: &str, dsl_result: Option<&DslParseResult>) -> RenderedOutput {
        *self.render_content.lock().unwrap() = Some(content.to_string());
        *self.render_dsl_result.lock().unwrap() = dsl_result.cloned();
        self.output.clone()
    }
}

/// MockAdapter that tracks send_message and send_card_json calls.
#[derive(Debug, Default)]
struct TrackingAdapter {
    send_message_called: Mutex<bool>,
    send_card_json_called: Mutex<bool>,
}

#[async_trait]
impl IMAdapter for TrackingAdapter {
    fn name(&self) -> &str {
        "tracking"
    }
    async fn handle_webhook(
        &self,
        _payload: &[u8],
    ) -> Result<Message, closeclaw::im::AdapterError> {
        Ok(Message {
            id: "1".into(),
            from: "a".into(),
            to: "b".into(),
            content: "hi".into(),
            channel: "tracking".into(),
            timestamp: 0,
            metadata: HashMap::new(),
        })
    }
    async fn send_message(&self, _message: &Message) -> Result<(), closeclaw::im::AdapterError> {
        *self.send_message_called.lock().unwrap() = true;
        Ok(())
    }
    async fn validate_signature(&self, _: &str, _: &[u8]) -> bool {
        true
    }
    async fn send_card_json(&self, _: &str, _: &str) -> Result<(), closeclaw::im::AdapterError> {
        *self.send_card_json_called.lock().unwrap() = true;
        Ok(())
    }
}

use gateway_send_outbound_basic::{make_config, make_outbound_gw, make_outbound_message};

#[tokio::test]
async fn test_send_outbound_with_renderer_text() {
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
            name: "clean".into(),
            output_content: "plain text content".to_string(),
            output_suppress: false,
            dsl_result_json: None,
        }));
        r
    });
    let renderer = Arc::new(MockRenderer::new(
        "text",
        serde_json::json!({"msg_type": "text", "content": {"text": "rendered text"}}),
    ));
    let gw = Gateway::with_renderer(config, Arc::clone(&sm), renderer.clone(), Some(registry));
    let adapter = Arc::new(TrackingAdapter::default());
    gw.register_adapter("tracking".into(), adapter.clone())
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert_eq!(
        renderer.render_content.lock().unwrap().clone(),
        Some("plain text content".to_string())
    );
    assert!(renderer.render_dsl_result.lock().unwrap().is_none());
    assert!(*adapter.send_message_called.lock().unwrap());
    assert!(!*adapter.send_card_json_called.lock().unwrap());
}

#[tokio::test]
async fn test_send_outbound_with_renderer_interactive() {
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
            name: "rich".into(),
            output_content: "# Title\nBody content".to_string(),
            output_suppress: false,
            dsl_result_json: None,
        }));
        r
    });
    let renderer = Arc::new(MockRenderer::new(
        "interactive",
        serde_json::json!({"msg_type": "interactive", "card": {"elements": []}}),
    ));
    let gw = Gateway::with_renderer(config, Arc::clone(&sm), renderer.clone(), Some(registry));
    let adapter = Arc::new(TrackingAdapter::default());
    gw.register_adapter("tracking".into(), adapter.clone())
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert!(*adapter.send_card_json_called.lock().unwrap());
    assert!(!*adapter.send_message_called.lock().unwrap());
}

#[tokio::test]
async fn test_send_outbound_with_renderer_dsl_result_passed() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let dsl_json = serde_json::to_string(&DslParseResult {
        clean_content: "Clean content".to_string(),
        instructions: vec![],
    })
    .unwrap();
    let registry = Arc::new({
        let mut r = closeclaw::processor_chain::ProcessorRegistry::new();
        r.register(Arc::new(MockOutboundProcessor {
            name: "with_dsl".into(),
            output_content: "Raw with DSL".to_string(),
            output_suppress: false,
            dsl_result_json: Some(dsl_json),
        }));
        r
    });
    let renderer = Arc::new(MockRenderer::new(
        "text",
        serde_json::json!({"msg_type": "text", "content": {"text": "rendered"}}),
    ));
    let gw = Gateway::with_renderer(config, Arc::clone(&sm), renderer.clone(), Some(registry));
    gw.register_adapter("tracking".into(), Arc::new(TrackingAdapter::default()))
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    let captured_dsl = renderer.render_dsl_result.lock().unwrap().clone();
    assert!(captured_dsl.is_some());
    assert_eq!(
        captured_dsl.as_ref().unwrap().clean_content,
        "Clean content"
    );
}

#[tokio::test]
async fn test_send_outbound_no_renderer_fallback_text() {
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
            name: "plain".into(),
            output_content: "plain fallback".to_string(),
            output_suppress: false,
            dsl_result_json: None,
        }));
        r
    });
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    let adapter = Arc::new(TrackingAdapter::default());
    gw.register_adapter("tracking".into(), adapter.clone())
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert!(*adapter.send_message_called.lock().unwrap());
    assert!(!*adapter.send_card_json_called.lock().unwrap());
}

#[tokio::test]
async fn test_send_outbound_no_renderer_no_registry_bypass() {
    let config = make_config();
    let (gw, sm) = make_outbound_gw(config);
    let adapter = Arc::new(TrackingAdapter::default());
    gw.register_adapter("tracking".into(), adapter.clone())
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "bypass raw", vec![])
        .await
        .unwrap();

    assert!(*adapter.send_message_called.lock().unwrap());
    assert!(!*adapter.send_card_json_called.lock().unwrap());
}

#[tokio::test]
async fn test_send_outbound_renderer_with_suppress() {
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
            output_content: "ignored".to_string(),
            output_suppress: true,
            dsl_result_json: None,
        }));
        r
    });
    let renderer = Arc::new(MockRenderer::new(
        "text",
        serde_json::json!({"msg_type": "text", "content": {"text": "should not send"}}),
    ));
    let gw = Gateway::with_renderer(config, Arc::clone(&sm), renderer.clone(), Some(registry));
    let adapter = Arc::new(TrackingAdapter::default());
    gw.register_adapter("tracking".into(), adapter.clone())
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert!(!*adapter.send_message_called.lock().unwrap());
    assert!(!*adapter.send_card_json_called.lock().unwrap());
    assert!(renderer.render_content.lock().unwrap().is_none());
}

#[tokio::test]
async fn test_send_outbound_renderer_requires_registry() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let renderer = Arc::new(MockRenderer::new(
        "text",
        serde_json::json!({"msg_type": "text", "content": {"text": "x"}}),
    ));
    let gw = Gateway::with_renderer(config, Arc::clone(&sm), renderer, None);
    gw.register_adapter("tracking".into(), Arc::new(TrackingAdapter::default()))
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    let result = gw.send_outbound(&sid, "tracking", "raw", vec![]).await;
    assert!(matches!(result, Err(GatewayError::OutboundError(_))));
}
