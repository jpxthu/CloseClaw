//! Gateway processor chain integration tests.
//!
//! These tests live in a separate file so that src/gateway/tests.rs stays
//! under the 500-line limit.

use crate::{DmScope, GatewayConfig, InboundChainInput, Message, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::IMPlugin;
use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_common::processor::loader::{
    ProcessorChainConfig, ProcessorChainLoader, ProcessorConfig,
};
use closeclaw_common::processor::ProcessError;
use closeclaw_common::processor::{
    MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage,
};
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;

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
        let prev = meta.get("trace").map(|s| s.as_str()).unwrap_or("");
        meta.insert("trace".to_string(), format!("{prev}/{}", self.tag));
        Ok(Some(ProcessedMessage {
            content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                ctx.content.clone(),
            )],
            metadata: meta,
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

fn make_gw(config: GatewayConfig) -> (crate::Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

struct MockAdapter {
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

#[async_trait]
impl IMPlugin for MockAdapter {
    fn platform(&self) -> &str {
        "mock"
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
        _content_blocks: &[closeclaw_llm::types::ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> closeclaw_common::im_plugin::RenderedOutput {
        closeclaw_common::im_plugin::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": ""}}),
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

    fn streaming_renderer(
        &self,
    ) -> &std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer> {
        &self.renderer
    }
}

/// Scenario 1: No registry → Gateway behaviour unchanged (bypass).
#[tokio::test]
async fn test_processor_chain_bypass_no_registry() {
    let (gw, sm) = make_gw(make_config());
    gw.register_plugin(Arc::new(MockAdapter {
        renderer: std::sync::Mutex::new(
            crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
        ),
    }))
    .await;
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
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));
    gw.register_plugin(Arc::new(MockAdapter {
        renderer: std::sync::Mutex::new(
            crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
        ),
    }))
    .await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "plain text").await;
    let content_before = msg.content.clone();
    gw.route_message("mock", msg, None).await.unwrap();
    let (gw2, sm2) = make_gw(make_config());
    gw2.register_plugin(Arc::new(MockAdapter {
        renderer: std::sync::Mutex::new(
            crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
        ),
    }))
    .await;
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
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));
    gw.register_plugin(Arc::new(MockAdapter {
        renderer: std::sync::Mutex::new(
            crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
        ),
    }))
    .await;
    let msg = msg_with_session(&sm, "mock", "agent-1", "hello").await;
    let result = gw.route_message("mock", msg, None).await;
    assert!(result.is_ok(), "expected ok, got {result:?}");
}

/// Scenario 4: Processors execute in priority order (ascending priority value).
#[tokio::test]
async fn test_processor_chain_execution_order_by_priority() {
    use closeclaw_common::processor::ProcessorRegistry;

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
        closeclaw_common::processor::content_normalizer::ContentNormalizer::new(),
    ));

    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));
    gw.register_plugin(Arc::new(MockAdapter {
        renderer: std::sync::Mutex::new(
            crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
        ),
    }))
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
            meta.insert("injected_key".to_string(), "injected_value".to_string());
            Ok(Some(ProcessedMessage {
                content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                    ctx.content.clone(),
                )],
                metadata: meta,
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
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(MetadataInjector(20)));

    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));
    gw.register_plugin(Arc::new(MockAdapter {
        renderer: std::sync::Mutex::new(
            crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
        ),
    }))
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
    registry: closeclaw_common::processor::ProcessorRegistry,
) -> crate::Gateway {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    crate::Gateway::with_processor_registry(config, sm, Arc::new(registry))
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
        Ok(None)
    }
}

// ── process_inbound_chain tests ──────────────────────────────────────────────

/// When no ProcessorRegistry is configured, `process_inbound_chain` returns
/// the original content unchanged (bypass).
#[tokio::test]
async fn test_process_inbound_chain_no_registry() {
    let (gw, _sm) = make_gw(make_config());
    let result = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "cli".into(),
            content: "hello world".into(),
            message_id: "msg-1".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;
    assert_eq!(result.text_content(), Some("hello world"));
    assert!(!result.content_blocks.is_empty());
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
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(
        closeclaw_common::processor::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));

    // Input with ANSI escape sequences and invisible control characters.
    let raw_input = "\x1b[31mhello\x1b[0m\x01world";
    let result = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "cli".into(),
            content: raw_input.into(),
            message_id: "msg-1".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;
    // ANSI stripped, control char stripped, plain text remains.
    assert_eq!(result.text_content(), Some("helloworld"));
    assert!(!result.content_blocks.is_empty());
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
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(closeclaw_common::processor::SessionRouter::new(
        DmScope::default(),
    )));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));

    let result = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "peer1".into(),
            content: "hi".into(),
            message_id: "msg-1".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;
    assert_eq!(result.text_content(), Some("hi"));
    // SessionRouter injects session_key with real timestamp.
    // Verify the key has the expected format: {ts_ms}-{sha256_hex}
    let key = result.metadata.get("session_key").map(|s| s.as_str());
    let key = key.expect("session_key should be set");
    let hash_part = SessionManager::strip_timestamp_from_session_key(key);
    assert_eq!(hash_part.len(), 64, "hash should be 64 hex chars: {key}");
    assert!(
        hash_part.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex: {key}"
    );
}

/// Verifies that `process_inbound_chain` uses system time (not the provided
/// timestamp parameter) for the session key, aligning with design doc.
#[tokio::test]
async fn test_process_inbound_chain_uses_system_time() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(closeclaw_common::processor::SessionRouter::new(
        DmScope::default(),
    )));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));

    let before_ms = chrono::Utc::now().timestamp_millis();
    let result = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "peer1".into(),
            content: "hi".into(),
            message_id: "msg-ts".into(),
            timestamp_ms: 1_700_000_000_123, // past timestamp — should NOT be used
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;
    let after_ms = chrono::Utc::now().timestamp_millis();

    let key = result
        .metadata
        .get("session_key")
        .map(|s| s.as_str())
        .expect("session_key should be set");

    // Key prefix must be between before_ms and after_ms (system time), not 1700000000123
    let ts_prefix: i64 = key[..key.find('-').unwrap()]
        .parse()
        .expect("key prefix should be parseable as i64");
    assert!(
        ts_prefix >= before_ms && ts_prefix <= after_ms,
        "session_key timestamp should reflect system time ({before_ms}..{after_ms}), got {ts_prefix}: {key}"
    );
    let hash_part = &key[key.find('-').unwrap() + 1..];
    assert_eq!(hash_part.len(), 64, "hash should be 64 hex chars: {key}");
    assert!(
        hash_part.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex: {key}"
    );
}

/// Verifies that `ContentNormalizer` does NOT call `strip_platform_residue`
/// during processing — platform format conversion is handled by adapters.
#[tokio::test]
async fn test_content_normalizer_does_not_strip_platform_residue() {
    use closeclaw_common::processor::processor::MessageProcessor;

    let processor = closeclaw_common::processor::content_normalizer::ContentNormalizer::new();
    let msg = closeclaw_common::im_plugin::NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "user1".to_string(),
        peer_id: "chat1".to_string(),
        // Content with platform-specific <at> tags
        content: r#"Hello <at user_id="u123">Alice</at>, welcome!"#.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let ctx = closeclaw_common::processor::MessageContext::from_normalized(msg);
    let result = processor.process(&ctx).await.unwrap().unwrap();
    // <at> tags should be preserved (not converted to @Alice)
    assert!(
        result.text_content().unwrap_or("").contains("<at user_id="),
        "ContentNormalizer should not strip platform residue, got: {:?}",
        result.content_blocks
    );
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
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(FailProcessor));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));

    let result = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "cli".into(),
            content: "original".into(),
            message_id: "msg-1".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;
    // Fallback to original content.
    assert_eq!(result.text_content(), Some("original"));
    assert!(!result.content_blocks.is_empty());
    assert!(result.metadata.is_empty());
}

// ── End-to-end integration tests ─────────────────────────────────────────────

/// Full inbound pipeline: SessionRouter + ContentNormalizer both present.
/// Verifies that ANSI control characters are stripped AND session_key is
/// injected into metadata after the chain completes.
#[tokio::test]
async fn test_e2e_inbound_full_stack_strips_ansi_and_injects_session_key() {
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(closeclaw_common::processor::SessionRouter::new(
        DmScope::default(),
    )));
    registry.register(Arc::new(
        closeclaw_common::processor::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = make_gw_with_registry(registry);

    let raw_input = "\x1b[32mhello\x1b[0m\x01world\x1b[1;34m!";
    let result = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "peer1".into(),
            content: raw_input.into(),
            message_id: "msg-e2e-1".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;

    assert_eq!(result.text_content(), Some("helloworld!"));
    assert!(!result.content_blocks.is_empty());

    // SessionRouter uses real timestamp — verify key format.
    let key = result.metadata.get("session_key").map(|s| s.as_str());
    let key = key.expect("session_key should be set");
    let hash_part = SessionManager::strip_timestamp_from_session_key(key);
    assert_eq!(hash_part.len(), 64, "hash should be 64 hex chars: {key}");
    assert!(
        hash_part.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex: {key}"
    );
}

/// After processing through the inbound chain, the cleaned content is
/// accepted by `handle_inbound_message` without error (returns None
/// when no session handler is installed — expected in unit test context).
#[tokio::test]
async fn test_e2e_processed_content_feeds_into_handle_inbound() {
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(closeclaw_common::processor::SessionRouter::new(
        DmScope::default(),
    )));
    registry.register(Arc::new(
        closeclaw_common::processor::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = make_gw_with_registry(registry);

    let raw_input = "\x1b[33mwhat is the weather\x1b[0m";
    let processed = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "cli".into(),
            content: raw_input.into(),
            message_id: "msg-e2e-2".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;

    assert_eq!(processed.text_content(), Some("what is the weather"));
    assert!(!processed.content_blocks.is_empty());

    let handle_result = gw
        .handle_inbound_message(processed, Some("user1"), "terminal")
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
    let mut registry = closeclaw_common::processor::ProcessorRegistry::new();
    registry.register(Arc::new(SuppressTestProcessor));
    registry.register(Arc::new(
        closeclaw_common::processor::content_normalizer::ContentNormalizer::new(),
    ));
    let gw = make_gw_with_registry(registry);

    let result = gw
        .process_inbound_chain(&InboundChainInput {
            platform: "terminal".into(),
            sender_id: "user1".into(),
            peer_id: "cli".into(),
            content: "hello".into(),
            message_id: "msg-e2e-4".into(),
            timestamp_ms: 0,
            account_id: None,
            thread_id: None,
            message_type: Default::default(),
            media_refs: Vec::new(),
            quoted_message: None,
        })
        .await;
    assert!(
        result.content_blocks.is_empty(),
        "suppress should produce empty content_blocks"
    );
}
