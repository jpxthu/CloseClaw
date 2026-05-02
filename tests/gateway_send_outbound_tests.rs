//! Tests for Gateway::send_outbound and related functionality (issue #469).
//!
//! These tests live here to keep src/gateway/tests.rs under 500 lines.

use async_trait::async_trait;
use closeclaw::gateway::{DmScope, Gateway, GatewayConfig, GatewayError, Message, SessionManager};
use closeclaw::im::{AdapterError, IMAdapter};
use closeclaw::processor_chain::{
    MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage,
};
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
            metadata: Default::default(),
            suppress: self.output_suppress,
        }))
    }
}

/// MockAdapter that tracks send_message and send_card_json calls.
#[derive(Debug, Default)]
struct TrackingAdapter {
    send_message_called: Mutex<bool>,
    send_card_json_called: Mutex<bool>,
    should_fail: Mutex<bool>,
}

#[async_trait]
impl IMAdapter for TrackingAdapter {
    fn name(&self) -> &str {
        "tracking"
    }

    async fn handle_webhook(&self, _payload: &[u8]) -> Result<Message, AdapterError> {
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

    async fn send_message(&self, _message: &Message) -> Result<(), AdapterError> {
        if *self.should_fail.lock().unwrap() {
            return Err(AdapterError::SendFailed("mock".into()));
        }
        *self.send_message_called.lock().unwrap() = true;
        Ok(())
    }

    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        true
    }

    async fn send_card_json(&self, _chat_id: &str, _card_json: &str) -> Result<(), AdapterError> {
        if *self.should_fail.lock().unwrap() {
            return Err(AdapterError::SendFailed("mock".into()));
        }
        *self.send_card_json_called.lock().unwrap() = true;
        Ok(())
    }
}

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
    }
}

fn make_outbound_message(to: &str, content: &str) -> Message {
    Message {
        id: "msg_1".to_string(),
        from: "ou_sender".to_string(),
        to: to.to_string(),
        content: content.to_string(),
        channel: "tracking".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    }
}

fn make_outbound_gw(config: GatewayConfig) -> (Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(&config, None));
    let gw = Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

#[tokio::test]
async fn test_send_outbound_no_registry_bypass() {
    let config = make_config();
    let (gw, sm) = make_outbound_gw(config);
    gw.register_adapter("tracking".into(), Arc::new(TrackingAdapter::default()))
        .await;

    // Create a session so get_chat_id returns a valid chat_id
    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    // No registry → raw_output sent as plain text via send_message
    gw.send_outbound(&sid, "tracking", "raw output")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_send_outbound_text_path() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(&config, None));
    let registry = Arc::new({
        let mut r = closeclaw::processor_chain::ProcessorRegistry::new();
        r.register(Arc::new(MockOutboundProcessor {
            name: "text_processor".into(),
            output_content: r#"{"msg_type":"text","content":"processed text"}"#.into(),
            output_suppress: false,
        }));
        r
    });
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    gw.register_adapter("tracking".into(), Arc::new(TrackingAdapter::default()))
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    // Registry returns text → adapter.send_message called
    gw.send_outbound(&sid, "tracking", "raw output")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_send_outbound_interactive_path() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(&config, None));
    let registry = Arc::new({
        let mut r = closeclaw::processor_chain::ProcessorRegistry::new();
        r.register(Arc::new(MockOutboundProcessor {
            name: "card_processor".into(),
            output_content: r#"{"msg_type":"interactive","content":"{\"elements\":[]}"}"#.into(),
            output_suppress: false,
        }));
        r
    });
    let gw = Gateway::with_processor_registry(config, Arc::clone(&sm), registry);
    let adapter = Arc::new(TrackingAdapter::default());
    gw.register_adapter("tracking".into(), adapter.clone())
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    gw.send_outbound(&sid, "tracking", "raw output")
        .await
        .unwrap();

    // Interactive path → send_card_json should be called, not send_message
    assert!(
        *adapter.send_card_json_called.lock().unwrap(),
        "send_card_json should be called for interactive msg_type"
    );
    assert!(
        !*adapter.send_message_called.lock().unwrap(),
        "send_message should NOT be called for interactive msg_type",
    );
}

#[tokio::test]
async fn test_send_outbound_suppress() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(&config, None));
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
    let adapter = Arc::new(TrackingAdapter::default());
    gw.register_adapter("tracking".into(), adapter.clone())
        .await;

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    // suppress=true → returns Ok without sending anything
    gw.send_outbound(&sid, "tracking", "raw output")
        .await
        .unwrap();

    assert!(
        !*adapter.send_message_called.lock().unwrap(),
        "send_message should not be called when suppress=true",
    );
    assert!(
        !*adapter.send_card_json_called.lock().unwrap(),
        "send_card_json should not be called when suppress=true",
    );
}

#[tokio::test]
async fn test_send_outbound_unknown_session() {
    let (gw, _sm) = make_outbound_gw(make_config());
    gw.register_adapter("tracking".into(), Arc::new(TrackingAdapter::default()))
        .await;

    // No session exists with this id
    let result = gw
        .send_outbound("nonexistent-session", "tracking", "raw")
        .await;
    assert!(matches!(result, Err(GatewayError::MissingSessionId)));
}

#[tokio::test]
async fn test_send_outbound_unknown_channel() {
    let (gw, sm) = make_outbound_gw(make_config());
    // No adapter registered for channel "unknown"

    let msg = make_outbound_message("agent-1", "hello");
    let sid = sm.find_or_create("tracking", &msg, None).await.unwrap();

    let result = gw.send_outbound(&sid, "unknown", "raw").await;
    assert!(matches!(result, Err(GatewayError::UnknownChannel(_))));
}

#[tokio::test]
async fn test_feishu_adapter_send_card_json_default() {
    // The default implementation returns UnsupportedOperation
    struct DummyAdapter;
    #[async_trait]
    impl IMAdapter for DummyAdapter {
        fn name(&self) -> &str {
            "dummy"
        }
        async fn handle_webhook(&self, _: &[u8]) -> Result<Message, AdapterError> {
            Err(AdapterError::InvalidPayload("x".into()))
        }
        async fn send_message(&self, _: &Message) -> Result<(), AdapterError> {
            Err(AdapterError::InvalidPayload("x".into()))
        }
        async fn validate_signature(&self, _: &str, _: &[u8]) -> bool {
            true
        }
        // Do NOT implement send_card_json → uses default
    }
    let adapter = DummyAdapter;
    let result = adapter.send_card_json("chat_1", "{}").await;
    assert!(matches!(result, Err(AdapterError::UnsupportedOperation)));
}
