//! Tests for Gateway::send_outbound — plugin integration tests (issue #829).
//!
//! Verifies that the unified plugin path (render → send) works correctly
//! for text, interactive, suppress, and dsl_result scenarios.

mod gateway_send_outbound_basic;

use async_trait::async_trait;
use closeclaw::processor_chain::{
    MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage,
};
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::InboundEvent;
use closeclaw_gateway::{Gateway, GatewayError, SessionManager};
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::{Arc, Mutex};

use gateway_send_outbound_basic::{make_config, make_outbound_gw, make_outbound_message};

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

/// MockOutboundProcessor — test processor for the outbound chain.
#[derive(Debug)]
struct MockOutboundProcessor {
    name: String,
    output_content: String,
    output_suppress: bool,
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
        if self.output_suppress {
            return Ok(None);
        }
        let mut metadata = std::collections::HashMap::new();
        if let Some(ref json) = self.dsl_result_json {
            metadata.insert("dsl_result".to_string(), json.clone());
        }
        Ok(Some(ProcessedMessage {
            content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                self.output_content.clone(),
            )],
            metadata,
        }))
    }
}

/// TrackingPlugin — records render and send calls for assertion.
///
/// `render()` returns a preconfigured [`RenderedOutput`].
/// `send()` records that it was called.
struct TrackingPlugin {
    platform_name: String,
    /// Captured render inputs: (content text from first Text block, dsl_result)
    render_called: Mutex<Option<(String, Option<DslParseResult>)>>,
    /// Whether `send()` was called
    send_called: Mutex<bool>,
    /// The RenderedOutput returned by `render()`
    render_output: RenderedOutput,
}

impl TrackingPlugin {
    fn new(platform: &str, msg_type: &str, payload: serde_json::Value) -> Self {
        Self {
            platform_name: platform.to_string(),
            render_called: Mutex::new(None),
            send_called: Mutex::new(false),
            render_output: RenderedOutput {
                msg_type: msg_type.to_string(),
                payload,
            },
        }
    }
}

#[async_trait]
impl IMPlugin for TrackingPlugin {
    fn platform(&self) -> &str {
        &self.platform_name
    }

    async fn parse_inbound(&self, _payload: &[u8]) -> Result<Option<InboundEvent>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        // Capture the text from the first Text block for assertion.
        let text = content_blocks
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();
        *self.render_called.lock().unwrap() = Some((text, dsl_result.cloned()));
        self.render_output.clone()
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_registry_with_processor(
    processor: MockOutboundProcessor,
) -> Arc<closeclaw::processor_chain::ProcessorRegistry> {
    Arc::new({
        let mut r = closeclaw::processor_chain::ProcessorRegistry::new();
        r.register(Arc::new(processor));
        r
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_plugin_text_path() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let registry = make_registry_with_processor(MockOutboundProcessor {
        name: "clean".into(),
        output_content: "plain text content".to_string(),
        output_suppress: false,
        dsl_result_json: None,
    });
    let plugin = Arc::new(TrackingPlugin::new(
        "tracking",
        "text",
        serde_json::json!({"content": {"text": "rendered text"}}),
    ));
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    // render() was called with the processor's output content
    let render = plugin.render_called.lock().unwrap().clone();
    let (ref text, ref dsl) = render.as_ref().expect("render should have been called");
    assert_eq!(text, "plain text content");
    assert!(dsl.is_none());

    // send() was called
    assert!(*plugin.send_called.lock().unwrap());
}

#[tokio::test]
async fn test_plugin_interactive_path() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let registry = make_registry_with_processor(MockOutboundProcessor {
        name: "rich".into(),
        output_content: "# Title\nBody".to_string(),
        output_suppress: false,
        dsl_result_json: None,
    });
    let plugin = Arc::new(TrackingPlugin::new(
        "tracking",
        "interactive",
        serde_json::json!({"card": {"elements": []}}),
    ));
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    assert!(*plugin.send_called.lock().unwrap());
}

#[tokio::test]
async fn test_plugin_dsl_result_passed_to_render() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let dsl_json = serde_json::to_string(&DslParseResult {
        instructions: vec![],
    })
    .unwrap();
    let registry = make_registry_with_processor(MockOutboundProcessor {
        name: "with_dsl".into(),
        output_content: "Raw with DSL".to_string(),
        output_suppress: false,
        dsl_result_json: Some(dsl_json),
    });
    let plugin = Arc::new(TrackingPlugin::new(
        "tracking",
        "text",
        serde_json::json!({"content": {"text": "rendered"}}),
    ));
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    let render = plugin.render_called.lock().unwrap().clone();
    let (_, ref dsl) = render.as_ref().expect("render should have been called");
    let dsl = dsl.as_ref().expect("dsl_result should have been passed");
    assert!(dsl.instructions.is_empty());
}

#[tokio::test]
async fn test_plugin_suppress_skips_send() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let registry = make_registry_with_processor(MockOutboundProcessor {
        name: "suppressor".into(),
        output_content: "ignored".to_string(),
        output_suppress: true,
        dsl_result_json: None,
    });
    let plugin = Arc::new(TrackingPlugin::new(
        "tracking",
        "text",
        serde_json::json!({"content": {"text": "should not send"}}),
    ));
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output", vec![])
        .await
        .unwrap();

    // Suppress → neither render nor send should be called
    assert!(plugin.render_called.lock().unwrap().is_none());
    assert!(!*plugin.send_called.lock().unwrap());
}

#[tokio::test]
async fn test_no_registry_bypass_uses_plugin() {
    let config = make_config();
    let (gw, sm) = make_outbound_gw(config);
    let plugin = Arc::new(TrackingPlugin::new(
        "tracking",
        "text",
        serde_json::json!({"content": {"text": "bypass"}}),
    ));
    gw.register_plugin(plugin.clone()).await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "bypass raw", vec![])
        .await
        .unwrap();

    // Even without a processor registry, the plugin should be used
    assert!(plugin.render_called.lock().unwrap().is_some());
    assert!(*plugin.send_called.lock().unwrap());
}

#[tokio::test]
async fn test_unknown_channel_returns_error() {
    let config = make_config();
    let (gw, sm) = make_outbound_gw(config);
    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    let result = gw.send_outbound(&sid, "unknown", "raw", vec![]).await;
    assert!(matches!(result, Err(GatewayError::UnknownChannel(_))));
}

#[tokio::test]
async fn test_unknown_session_returns_error() {
    let config = make_config();
    let (gw, _sm) = make_outbound_gw(config);
    let plugin = Arc::new(TrackingPlugin::new(
        "tracking",
        "text",
        serde_json::json!({"content": {"text": "x"}}),
    ));
    gw.register_plugin(plugin).await;

    let result = gw
        .send_outbound("nonexistent-session", "tracking", "raw", vec![])
        .await;
    assert!(matches!(result, Err(GatewayError::MissingSessionId)));
}
