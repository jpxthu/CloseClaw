//! Gateway processor chain integration tests.
//!
//! These tests live in a separate file so that src/gateway/tests.rs stays
//! under the 500-line limit.

use crate::gateway::{DmScope, GatewayConfig, Message, SessionManager};
use crate::im::IMPlugin;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::processor_chain::error::ProcessError;
use crate::processor_chain::loader::{ProcessorChainConfig, ProcessorChainLoader, ProcessorConfig};
use crate::processor_chain::{MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;

// ── Test helpers ─────────────────────────────────────────────────────────────

/// A test processor that appends its name and phase to a metadata key,
/// proving execution order. Runs at priority 20 (between RawLog=10 and
/// ContentNormalizer=30) so that including it in a chain changes the
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
            content_blocks: vec![],
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
        ..Default::default()
    }
}

fn make_gw(config: GatewayConfig) -> (crate::gateway::Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

struct MockAdapter;

#[async_trait]
impl IMPlugin for MockAdapter {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<crate::im::NormalizedMessage>, crate::im::AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        _content_blocks: &[crate::llm::types::ContentBlock],
        _dsl_result: Option<&crate::processor_chain::DslParseResult>,
    ) -> crate::renderer::RenderedOutput {
        crate::renderer::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": ""}}),
        }
    }

    async fn send(
        &self,
        _output: &crate::renderer::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), crate::im::AdapterError> {
        Ok(())
    }
}

/// Scenario 1: No registry → Gateway behaviour unchanged (bypass).
#[tokio::test]
async fn test_processor_chain_bypass_no_registry() {
    let (gw, sm) = make_gw(make_config());
    gw.register_plugin(Arc::new(MockAdapter)).await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "plain text").await;
    let result = gw.route_message("mock", msg, None).await;
    assert!(result.is_ok(), "expected ok, got {result:?}");
}

/// Scenario 2: Empty registry (inbound empty) → behaviour identical to no registry.
#[tokio::test]
async fn test_processor_chain_bypass_empty_registry() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let registry = ProcessorChainLoader::load(&ProcessorChainConfig {
        inbound: vec![],
        outbound: vec![],
    })
    .unwrap();
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_plugin(Arc::new(MockAdapter)).await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "plain text").await;
    let content_before = msg.content.clone();
    gw.route_message("mock", msg, None).await.unwrap();
    let (gw2, sm2) = make_gw(make_config());
    gw2.register_plugin(Arc::new(MockAdapter)).await;
    let msg2 = msg_with_session(&sm2, "mock", "agent-1", "plain text").await;
    assert_eq!(msg2.content, content_before);
}

/// Scenario 3: Configured processors → message content is processed.
#[tokio::test]
async fn test_processor_chain_applies_processors() {
    let tmp = tempfile::tempdir().unwrap();
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let proc_config = ProcessorChainConfig {
        inbound: vec![ProcessorConfig::RawLog {
            enabled: true,
            dir: tmp.path().to_path_buf(),
            retention_days: 7,
        }],
        outbound: vec![],
    };
    let registry = ProcessorChainLoader::load(&proc_config).unwrap();
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_plugin(Arc::new(MockAdapter)).await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    let result = gw.route_message("mock", msg, None).await;
    assert!(result.is_ok(), "expected ok, got {result:?}");
}

/// Scenario 4: Processors execute in priority order (ascending priority value).
#[tokio::test]
async fn test_processor_chain_execution_order_by_priority() {
    use crate::processor_chain::ProcessorRegistry;

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));

    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(TraceProcessor {
        name: "trace_20".into(),
        tag: "T20".into(),
    }));
    registry.register(Arc::new(
        crate::processor_chain::content_normalizer::ContentNormalizer::new(),
    ));

    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_plugin(Arc::new(MockAdapter)).await;

    let raw_feishu = r#"{"msg_type":"text","text":{"text":"hello"}}"#;
    let mut msg = Message {
        id: "t".into(),
        from: "u1".into(),
        to: "a1".into(),
        content: raw_feishu.into(),
        channel: "mock".into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
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
                content_blocks: vec![],
            }))
        }
    }

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(MetadataInjector(20)));

    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_plugin(Arc::new(MockAdapter)).await;

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
        thread_id: None,
    }
}

async fn msg_with_session(sm: &SessionManager, channel: &str, to: &str, content: &str) -> Message {
    let mut msg = make_message(to, content);
    let sid = sm.find_or_create(channel, &msg, None).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);
    msg
}

/// Build a Gateway with the given ProcessorRegistry (shared across E2E tests).
fn make_gw_with_registry(
    registry: crate::processor_chain::ProcessorRegistry,
) -> crate::gateway::Gateway {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    crate::gateway::Gateway::with_processor_registry(config, sm, Arc::new(registry))
}

/// A processor that always suppresses the message (for E2E suppress tests).
struct SuppressTestProcessor;

#[async_trait]
impl MessageProcessor for SuppressTestProcessor {
    fn name(&self) -> &str {
        "suppress"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Inbound
    }

    fn priority(&self) -> u8 {
        10
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        Ok(Some(ProcessedMessage {
            content: ctx.content.clone(),
            metadata: ctx.metadata.clone(),
            suppress: true,
            content_blocks: vec![],
        }))
    }
}

// ── process_inbound_chain tests ──────────────────────────────────────────────

/// When no ProcessorRegistry is configured, `process_inbound_chain` returns
/// the original content unchanged (bypass).
#[tokio::test]
async fn test_process_inbound_chain_no_registry() {
    let (gw, _sm) = make_gw(make_config());
    let result = gw
        .process_inbound_chain("terminal", "user1", "cli", "hello world", "msg-1")
        .await;
    assert_eq!(result.content, "hello world");
    assert!(!result.suppress);
    assert!(result.metadata.is_empty());
}

/// ContentNormalizer registered in the chain strips ANSI control characters
/// from stdin input.
#[tokio::test]
async fn test_process_inbound_chain_with_normalizer() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(
        crate::processor_chain::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );

    // Input with ANSI escape sequences and invisible control characters.
    let raw_input = "\x1b[31mhello\x1b[0m\x01world";
    let result = gw
        .process_inbound_chain("terminal", "user1", "cli", raw_input, "msg-1")
        .await;
    // ANSI stripped, control char stripped, plain text remains.
    assert_eq!(result.content, "helloworld");
    assert!(!result.suppress);
}

/// SessionRouter in the chain computes and attaches `session_key` to metadata.
#[tokio::test]
async fn test_process_inbound_chain_with_session_router() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(crate::processor_chain::SessionRouter::new(
        DmScope::default(),
    )));
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );

    let result = gw
        .process_inbound_chain("terminal", "user1", "peer1", "hi", "msg-1")
        .await;
    assert_eq!(result.content, "hi");
    // SessionRouter injects session_key = "default:terminal:user1:peer1" (PerAccountChannelPeer)
    let key = result.metadata.get("session_key").and_then(|v| v.as_str());
    assert_eq!(key, Some("default:terminal:user1:peer1"));
}

/// When a processor returns an error, `process_inbound_chain` falls back to
/// the original content and logs a warning.
#[tokio::test]
async fn test_process_inbound_chain_processor_error() {
    /// A processor that always returns an error.
    struct FailProcessor;

    #[async_trait]
    impl MessageProcessor for FailProcessor {
        fn name(&self) -> &str {
            "fail"
        }

        fn phase(&self) -> ProcessPhase {
            ProcessPhase::Inbound
        }

        fn priority(&self) -> u8 {
            0
        }

        async fn process(
            &self,
            _ctx: &MessageContext,
        ) -> Result<Option<ProcessedMessage>, ProcessError> {
            Err(ProcessError::processor_failed("fail", "deliberate"))
        }
    }

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(FailProcessor));
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );

    let result = gw
        .process_inbound_chain("terminal", "user1", "cli", "original", "msg-1")
        .await;
    // Fallback to original content.
    assert_eq!(result.content, "original");
    assert!(!result.suppress);
    assert!(result.metadata.is_empty());
}

// ── End-to-end integration tests ─────────────────────────────────────────────

/// Full inbound pipeline: SessionRouter + ContentNormalizer both present.
/// Verifies that ANSI control characters are stripped AND session_key is
/// injected into metadata after the chain completes.
#[tokio::test]
async fn test_e2e_inbound_full_stack_strips_ansi_and_injects_session_key() {
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(crate::processor_chain::SessionRouter::new(
        DmScope::default(),
    )));
    registry.register(Arc::new(
        crate::processor_chain::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = make_gw_with_registry(registry);

    let raw_input = "\x1b[32mhello\x1b[0m\x01world\x1b[1;34m!";
    let result = gw
        .process_inbound_chain("terminal", "user1", "peer1", raw_input, "msg-e2e-1")
        .await;

    assert_eq!(result.content, "helloworld!");
    assert!(!result.suppress);

    let expected_key = DmScope::default().compute_session_key(
        "terminal",
        &Message {
            id: String::new(),
            from: "user1".to_string(),
            to: "peer1".to_string(),
            content: String::new(),
            channel: "terminal".to_string(),
            timestamp: 0,
            metadata: HashMap::new(),
            thread_id: None,
        },
        None,
    );
    let key = result.metadata.get("session_key").and_then(|v| v.as_str());
    assert_eq!(key, Some(expected_key.as_str()));
}

/// After processing through the inbound chain, the cleaned content is
/// accepted by `handle_inbound_message` without error (returns None
/// when no session handler is installed — expected in unit test context).
#[tokio::test]
async fn test_e2e_processed_content_feeds_into_handle_inbound() {
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(crate::processor_chain::SessionRouter::new(
        DmScope::default(),
    )));
    registry.register(Arc::new(
        crate::processor_chain::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = make_gw_with_registry(registry);

    let raw_input = "\x1b[33mwhat is the weather\x1b[0m";
    let processed = gw
        .process_inbound_chain("terminal", "user1", "cli", raw_input, "msg-e2e-2")
        .await;

    assert_eq!(processed.content, "what is the weather");
    assert!(!processed.suppress);

    let handle_result = gw
        .handle_inbound_message("sess-1", processed.content, Some("user1"), "terminal")
        .await;
    assert!(
        handle_result.is_none(),
        "expected None when no session handler installed"
    );
}

/// Verify that a suppress=true processor in the chain causes the final
/// processed message to have suppress set, and the cleaned content
/// is still available for inspection.
#[tokio::test]
async fn test_e2e_suppress_flag_propagates_through_chain() {
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(SuppressTestProcessor));
    registry.register(Arc::new(
        crate::processor_chain::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = make_gw_with_registry(registry);

    let result = gw
        .process_inbound_chain("terminal", "user1", "cli", "hello", "msg-e2e-4")
        .await;
    assert!(result.suppress, "suppress flag should propagate");
    assert_eq!(result.content, "hello");
}
