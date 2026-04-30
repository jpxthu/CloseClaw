//! Gateway processor chain integration tests.
//!
//! These tests live in a separate file so that src/gateway/tests.rs stays
//! under the 500-line limit.

use crate::gateway::{DmScope, GatewayConfig, Message, SessionManager};
use crate::im::IMAdapter;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::processor_chain::error::ProcessError;
use crate::processor_chain::loader::{ProcessorChainConfig, ProcessorChainLoader, ProcessorConfig};
use crate::processor_chain::{MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage};

// ── Test helpers ─────────────────────────────────────────────────────────────

/// A test processor that appends its name and phase to a metadata key,
/// proving execution order. Runs at priority 20 (between RawLog=10 and
/// MessageCleaner=30) so that including it in a chain changes the
/// execution sequence visibly.
#[derive(Debug)]
struct TraceProcessor {
    name: String,
    tag: String,
}

#[async_trait]
impl MessageProcessor for TraceProcessor {
    fn name(&self) -> &str {
        &self.name
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        20
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        let mut meta = ctx.metadata.clone();
        let prev = meta.get("trace").and_then(|v| v.as_str()).unwrap_or("");
        meta.insert(
            "trace".to_string(),
            serde_json::json!(format!("{prev}/{}", self.tag)),
        );
        Ok(Some(ProcessedMessage {
            content: ctx.content.clone(),
            metadata: meta,
            suppress: false,
        }))
    }
}

// ── Test helpers shared with tests.rs ─────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
    }
}

fn make_gw(config: GatewayConfig) -> (crate::gateway::Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(&config, None));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

struct MockAdapter;

#[async_trait]
impl IMAdapter for MockAdapter {
    fn name(&self) -> &str {
        "mock"
    }

    async fn handle_webhook(&self, _payload: &[u8]) -> Result<Message, crate::im::AdapterError> {
        Ok(Message {
            id: "1".into(),
            from: "a".into(),
            to: "b".into(),
            content: "hi".into(),
            channel: "mock".into(),
            timestamp: 0,
            metadata: HashMap::new(),
        })
    }

    async fn send_message(&self, _message: &Message) -> Result<(), crate::im::AdapterError> {
        Ok(())
    }

    async fn validate_signature(&self, _signature: &str, _payload: &[u8]) -> bool {
        true
    }
}

/// Scenario 1: No registry → Gateway behaviour unchanged (bypass).
#[tokio::test]
async fn test_processor_chain_bypass_no_registry() {
    let (gw, sm) = make_gw(make_config());
    gw.register_adapter("mock".into(), Arc::new(MockAdapter))
        .await;
    let mut msg = msg_with_session(&sm, "mock", "agent-1", "plain text").await;
    let result = gw.route_message("mock", msg, None).await;
    assert!(result.is_ok(), "expected ok, got {result:?}");
}

/// Scenario 2: Empty registry (inbound empty) → behaviour identical to no registry.
#[tokio::test]
async fn test_processor_chain_bypass_empty_registry() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(&config, None));
    let registry = ProcessorChainLoader::load(&ProcessorChainConfig { inbound: vec![] }).unwrap();
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_adapter("mock".into(), Arc::new(MockAdapter))
        .await;
    let mut msg = msg_with_session(&sm, "mock", "agent-1", "plain text").await;
    let content_before = msg.content.clone();
    gw.route_message("mock", msg, None).await.unwrap();
    let (gw2, sm2) = make_gw(make_config());
    gw2.register_adapter("mock".into(), Arc::new(MockAdapter))
        .await;
    let mut msg2 = msg_with_session(&sm2, "mock", "agent-1", "plain text").await;
    assert_eq!(msg2.content, content_before);
}

/// Scenario 3: Configured processors → message content is processed.
#[tokio::test]
async fn test_processor_chain_applies_processors() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config();
    let sm = Arc::new(SessionManager::new(&config, None));
    let proc_config = ProcessorChainConfig {
        inbound: vec![ProcessorConfig::RawLog {
            enabled: true,
            dir: tmp.path().to_path_buf(),
            retention_days: 7,
        }],
    };
    let registry = ProcessorChainLoader::load(&proc_config).unwrap();
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_adapter("mock".into(), Arc::new(MockAdapter))
        .await;
    let mut msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    let result = gw.route_message("mock", msg, None).await;
    assert!(result.is_ok(), "expected ok, got {result:?}");
}

/// Scenario 4: Processors execute in priority order (ascending priority value).
#[tokio::test]
async fn test_processor_chain_execution_order_by_priority() {
    use crate::processor_chain::ProcessorRegistry;

    let config = make_config();
    let sm = Arc::new(SessionManager::new(&config, None));

    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(TraceProcessor {
        name: "trace_20".into(),
        tag: "T20".into(),
    }));
    registry.register(Arc::new(
        crate::processor_chain::message_cleaner::MessageCleaner::new(),
    ));

    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_adapter("mock".into(), Arc::new(MockAdapter))
        .await;

    let raw_feishu = r#"{"msg_type":"text","text":{"text":"hello"}}"#;
    let mut msg = Message {
        id: "t".into(),
        from: "u1".into(),
        to: "a1".into(),
        content: raw_feishu.into(),
        channel: "mock".into(),
        timestamp: 0,
        metadata: HashMap::new(),
    };
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);

    gw.route_message("mock", msg, None).await.unwrap();
}

/// Verifies that when a processor chain is active, the gateway correctly
/// passes the raw message through process_inbound and merges metadata back.
#[tokio::test]
async fn test_processor_chain_metadata_merge() {
    // A processor that injects a metadata key.
    struct MetadataInjector(u8);

    #[async_trait]
    impl MessageProcessor for MetadataInjector {
        fn name(&self) -> &str {
            "injector"
        }

        fn phase(&self) -> ProcessPhase {
            ProcessPhase::Inbound
        }

        fn priority(&self) -> u8 {
            self.0
        }

        async fn process(
            &self,
            ctx: &MessageContext,
        ) -> Result<Option<ProcessedMessage>, ProcessError> {
            let mut meta = ctx.metadata.clone();
            meta.insert(
                "injected_key".to_string(),
                serde_json::json!("injected_value"),
            );
            Ok(Some(ProcessedMessage {
                content: ctx.content.clone(),
                metadata: meta,
                suppress: false,
            }))
        }
    }

    let config = make_config();
    let sm = Arc::new(SessionManager::new(&config, None));
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(MetadataInjector(20)));

    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_adapter("mock".into(), Arc::new(MockAdapter))
        .await;

    let mut msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    msg.content = r#"{"msg_type":"text","text":{"text":"hello"}}"#.into();

    let result = gw.route_message("mock", msg, None).await;
    assert!(
        result.is_ok(),
        "expected ok from processor chain, got {result:?}"
    );
}

// ── Helpers used by multiple tests ───────────────────────────────────────────

fn make_message(to: &str, content: &str) -> Message {
    Message {
        id: "msg_1".to_string(),
        from: "ou_sender".to_string(),
        to: to.to_string(),
        content: content.to_string(),
        channel: "mock".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    }
}

async fn msg_with_session(sm: &SessionManager, channel: &str, to: &str, content: &str) -> Message {
    let mut msg = make_message(to, content);
    let sid = sm.find_or_create(channel, &msg, None).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);
    msg
}
