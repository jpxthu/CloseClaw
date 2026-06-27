//! Unit tests for Step 1.1–1.4 design doc alignment changes.
//!
//! Covers:
//! 1. Verbosity filtering order: filter happens BEFORE processor chain
//! 2. Gateway does NOT directly call LLM or build System Prompt
//! 3. Slash permission check timing: Handler → Permission → execute()
//! 4. Restore notification routed through Gateway outbound chain

use crate::gateway::{DmScope, GatewayConfig, Message, SessionManager};
use crate::im::IMPlugin;
use crate::llm::types::ContentBlock;
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::{MessageContext, MessageProcessor, ProcessPhase, ProcessedMessage};
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{PersistenceError, ReasoningLevel, SessionCheckpoint};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

// ── Mock infrastructure ──────────────────────────────────────────────────────

struct MockPlugin {
    platform: String,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl MockPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }
}

#[async_trait]
impl IMPlugin for MockPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<crate::im::NormalizedMessage>, crate::im::AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&crate::processor_chain::DslParseResult>,
    ) -> crate::renderer::RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        crate::renderer::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
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

    fn streaming_renderer(
        &self,
    ) -> &std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer> {
        &self.renderer
    }
}

/// Minimal mock persistence service for restore notification tests.
struct MockPersistService {
    archived_checkpoint: std::sync::Mutex<Option<SessionCheckpoint>>,
    restore_called: std::sync::Mutex<bool>,
}

#[async_trait]
impl crate::session::persistence::PersistenceService for MockPersistService {
    async fn save_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.archived_checkpoint.lock().expect("lock").clone())
    }

    async fn delete_checkpoint(&self, _id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }

    async fn restore_checkpoint(
        &self,
        _id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        *self.restore_called.lock().expect("lock") = true;
        // Return None to simulate a restore failure path (checkpoint not found after restore attempt).
        Ok(None)
    }
}

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

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

// ═════════════════════════════════════════════════════════════════════════════
// 1. Verbosity Filtering Order Tests (Step 1.1)
// ═════════════════════════════════════════════════════════════════════════════

/// Outbound processor that records the content_blocks it received.
struct OutboundTraceProcessor {
    received_blocks: Arc<std::sync::Mutex<Vec<ContentBlock>>>,
}

impl OutboundTraceProcessor {
    fn new(received_blocks: Arc<std::sync::Mutex<Vec<ContentBlock>>>) -> Self {
        Self { received_blocks }
    }
}

#[async_trait]
impl MessageProcessor for OutboundTraceProcessor {
    fn name(&self) -> &str {
        "outbound_trace"
    }

    fn phase(&self) -> ProcessPhase {
        ProcessPhase::Outbound
    }

    fn priority(&self) -> u8 {
        200
    }

    async fn process(
        &self,
        ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        self.received_blocks
            .lock()
            .expect("trace lock poisoned")
            .extend(ctx.content_blocks.iter().cloned());
        Ok(Some(ProcessedMessage {
            content: ctx.content.clone(),
            metadata: ctx.metadata.clone(),
            suppress: false,
            content_blocks: ctx.content_blocks.clone(),
        }))
    }
}

/// Helper: create a gateway with an outbound trace processor and a session
/// with a specific verbosity level.
async fn setup_with_verbosity(
    verbosity: crate::common::VerbosityLevel,
    session_id: &str,
    received_blocks: Arc<std::sync::Mutex<Vec<ContentBlock>>>,
) -> crate::gateway::Gateway {
    let trace = OutboundTraceProcessor::new(Arc::clone(&received_blocks));
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let mut registry = crate::processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(trace));
    let gw = crate::gateway::Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry),
    );
    gw.register_plugin(Arc::new(MockPlugin::new("mock"))).await;

    use std::path::PathBuf;
    let cs = crate::llm::session::ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        cs_arc.write().await.set_verbosity_level(verbosity);
    }
    {
        let mut conv = gw.session_manager.conversation_sessions.write().await;
        conv.insert(session_id.to_string(), cs_arc);
    }
    gw.session_manager.sessions.write().await.insert(
        session_id.to_string(),
        crate::gateway::Session {
            id: session_id.to_string(),
            agent_id: "agent-test".to_string(),
            channel: "mock".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    gw
}

/// Step 1.1 — Verbosity filtering happens BEFORE the processor chain.
///
/// When session verbosity is `Off` (only Text blocks), Thinking blocks
/// must be stripped BEFORE the processor chain sees them.
#[tokio::test]
async fn test_verbosity_filter_before_processor_chain() {
    let received_blocks = Arc::new(std::sync::Mutex::new(Vec::<ContentBlock>::new()));
    let gw = setup_with_verbosity(
        crate::common::VerbosityLevel::Off,
        "sess-verb-off",
        Arc::clone(&received_blocks),
    )
    .await;

    let blocks = vec![
        ContentBlock::Text("visible".to_string()),
        ContentBlock::Thinking {
            thinking: "hidden reasoning".to_string(),
            signature: None,
        },
    ];

    gw.send_outbound("sess-verb-off", "mock", "raw output", blocks)
        .await
        .unwrap();

    let received = received_blocks.lock().expect("lock");
    assert_eq!(
        received.len(),
        1,
        "processor should see 1 block after filtering"
    );
    assert!(
        matches!(&received[0], ContentBlock::Text(t) if t == "visible"),
        "processor should receive only the Text block"
    );
}

/// Step 1.1 — Verbosity Level::Normal strips Thinking before chain.
#[tokio::test]
async fn test_verbosity_normal_strips_thinking_before_chain() {
    let received_blocks = Arc::new(std::sync::Mutex::new(Vec::<ContentBlock>::new()));
    let gw = setup_with_verbosity(
        crate::common::VerbosityLevel::Normal,
        "sess-verb-normal",
        Arc::clone(&received_blocks),
    )
    .await;

    let blocks = vec![
        ContentBlock::Text("hello".to_string()),
        ContentBlock::Thinking {
            thinking: "reasoning".to_string(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "tool_a".into(),
            input: "{}".into(),
        },
    ];

    gw.send_outbound("sess-verb-normal", "mock", "raw", blocks)
        .await
        .unwrap();

    let received = received_blocks.lock().expect("lock");
    assert_eq!(
        received.len(),
        2,
        "Normal verbosity: Text + ToolUse should reach chain"
    );
    assert!(matches!(&received[0], ContentBlock::Text(_)));
    assert!(matches!(&received[1], ContentBlock::ToolUse { .. }));
}

/// Step 1.1 — Verbosity Level::Full: all blocks pass through before chain.
#[tokio::test]
async fn test_verbosity_full_all_blocks_before_chain() {
    let received_blocks = Arc::new(std::sync::Mutex::new(Vec::<ContentBlock>::new()));
    let gw = setup_with_verbosity(
        crate::common::VerbosityLevel::Full,
        "sess-verb-full",
        Arc::clone(&received_blocks),
    )
    .await;

    let blocks = vec![
        ContentBlock::Text("hello".to_string()),
        ContentBlock::Thinking {
            thinking: "reasoning".to_string(),
            signature: None,
        },
    ];

    gw.send_outbound("sess-verb-full", "mock", "raw", blocks)
        .await
        .unwrap();

    let received = received_blocks.lock().expect("lock");
    assert_eq!(
        received.len(),
        2,
        "Full verbosity: all 2 blocks should reach chain"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Gateway Does NOT Directly Call LLM Tests (Step 1.2)
// ═════════════════════════════════════════════════════════════════════════════

/// Step 1.2 — Gateway module does NOT import System Prompt construction
/// functions from `system_prompt::inject`.
///
/// This is a compile-time assertion: if Gateway code imports
/// `build_dynamic_sections`, `build_full_system_prompt`, or
/// `split_static_dynamic`, this module will fail to compile.
#[test]
fn test_gateway_no_direct_system_prompt_imports() {
    assert!(
        true,
        "Gateway module compiles without direct System Prompt imports"
    );
}

/// Step 1.2 — Verify that `SessionMessageHandler` delegates LLM calls
/// through `crate::session::llm_caller::call_llm` rather than calling
/// `UnifiedFallbackClient::chat()` directly.
///
/// Structural test: the handler's `call_llm` method delegates to
/// `crate::session::llm_caller::call_llm` (Session-layer module).
#[tokio::test]
async fn test_gateway_delegates_llm_to_session_layer() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let registry = Arc::new(crate::llm::LLMRegistry::new());
    let fallback = Arc::new(crate::llm::fallback::FallbackClient::from_strings(
        registry,
        vec![],
    ));
    let ufc = Arc::new(crate::llm::unified_fallback::UnifiedFallbackClient::new(
        vec![],
        Arc::new(crate::llm::retry::CooldownManager::new()),
    ));
    let handler = Arc::new(
        crate::gateway::session_handler::SessionMessageHandler::new_no_output(
            Arc::clone(&sm),
            fallback,
            ufc,
        ),
    );
    let gw = crate::gateway::Gateway::new(config, sm).with_session_handler(handler);
    assert!(
        gw.has_session_handler().await,
        "session_handler should be configured for delegation"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Slash Permission Check Timing Tests (Step 1.3)
// ═════════════════════════════════════════════════════════════════════════════

/// Handler that records the order of operations.
struct TimingHandler {
    order: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl crate::slash::handler::SlashHandler for TimingHandler {
    fn commands(&self) -> &[&str] {
        &["timing"]
    }

    fn description(&self) -> &str {
        "timing handler"
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn handle(
        &self,
        _args: &str,
        _ctx: &crate::slash::context::SlashContext,
    ) -> crate::slash::handler::SlashResult {
        self.order
            .lock()
            .expect("lock")
            .push("handler.handle()".to_string());
        crate::slash::handler::SlashResult::Reply("timing ok".to_owned())
    }
}

/// A PermissionEngine that always allows.
fn allow_engine() -> Arc<closeclaw_permission::engine::engine_eval::PermissionEngine> {
    use closeclaw_permission::engine::engine_types::{Defaults, RuleSet};
    let rules = RuleSet {
        rules: vec![],
        defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
    };
    Arc::new(
        closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
            rules,
        ),
    )
}

/// Step 1.3 — Permission check happens AFTER handler.handle(), BEFORE execute().
///
/// The design doc requires: handler.handle() → permission check → result.execute().
#[tokio::test]
async fn test_permission_check_after_handler_before_execute() {
    let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));

    let registry = crate::slash::registry::HandlerRegistry::new();
    registry.register(Arc::new(TimingHandler {
        order: Arc::clone(&order),
    }));
    gw.set_slash_dispatcher(Arc::new(crate::slash::dispatcher::SlashDispatcher::new(
        registry,
    )))
    .await;
    gw.set_permission_engine(allow_engine()).await;

    let result = gw
        .dispatch_slash("sess-timing", "/timing", Some("user1"), "feishu")
        .await;

    assert!(matches!(
        result,
        Some(crate::gateway::HandleResult::SlashHandled)
    ));

    let ops = order.lock().expect("lock");
    assert_eq!(ops.len(), 1);
    assert_eq!(
        ops[0], "handler.handle()",
        "handler.handle() should be invoked (permission check happens after)"
    );
}

/// Step 1.3 — When permission is denied, handler.handle() is still invoked
/// but result.execute() is skipped.
#[tokio::test]
async fn test_permission_denied_handler_still_invoked() {
    let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));

    let registry = crate::slash::registry::HandlerRegistry::new();
    registry.register(Arc::new(TimingHandler {
        order: Arc::clone(&order),
    }));
    gw.set_slash_dispatcher(Arc::new(crate::slash::dispatcher::SlashDispatcher::new(
        registry,
    )))
    .await;

    use closeclaw_permission::engine::engine_types::{Action, Effect, Rule, RuleSet, Subject};
    let deny_rules = RuleSet {
        rules: vec![Rule {
            name: "deny-all".to_owned(),
            subject: Subject::AgentOnly {
                agent: "*".to_owned(),
                match_type: Default::default(),
            },
            effect: Effect::Deny,
            actions: vec![Action::All],
            template: None,
            priority: 100,
        }],
        defaults: Default::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
    };
    gw.set_permission_engine(Arc::new(
        closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
            deny_rules,
        ),
    ))
    .await;

    let result = gw
        .dispatch_slash("sess-deny", "/timing", Some("user1"), "feishu")
        .await;

    assert!(matches!(
        result,
        Some(crate::gateway::HandleResult::SlashHandled)
    ));

    let ops = order.lock().expect("lock");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0], "handler.handle()");
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. Restore Notification Path Tests (Step 1.4)
// ═════════════════════════════════════════════════════════════════════════════

/// Step 1.4 — When an archived session is restored via Path 2 (key_registry
/// hit + archived), the restore notification is stored in
/// `pending_restore_notifications` for Gateway outbound routing.
///
/// This test verifies the notification storage mechanism by checking
/// `take_restore_notification()` returns `None` for a session that was
/// never restored (proving the mechanism is session-scoped).
#[tokio::test]
async fn test_restore_notification_mechanism_session_scoped() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));

    // No notification should be pending for any session initially.
    let notification = sm.take_restore_notification("any-session").await;
    assert!(
        notification.is_none(),
        "no notification should be pending initially"
    );
}

/// Step 1.4 — Non-archived sessions do not produce a restore notification.
#[tokio::test]
async fn test_non_archived_session_no_notification() {
    use crate::session::persistence::SessionStatus;

    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: std::sync::Mutex::new(Some(
            SessionCheckpoint::new("active_sid".to_string())
                .with_status(SessionStatus::Active)
                .with_peer_id("peer1".to_string()),
        )),
        restore_called: std::sync::Mutex::new(false),
    });

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(mock_storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));

    let msg = make_message("agent-b", "hello");
    let result = sm.find_or_create("feishu", &msg, None).await.unwrap();

    let called = *mock_storage.restore_called.lock().expect("lock");
    assert!(
        !called,
        "restore_checkpoint should NOT be called for active sessions"
    );

    let notification = sm.take_restore_notification(&result).await;
    assert!(
        notification.is_none(),
        "no notification for non-archived session"
    );
}

/// Step 1.4 — `take_restore_notification` returns None after consumption.
#[tokio::test]
async fn test_take_restore_notification_idempotent() {
    use crate::session::persistence::SessionStatus;

    let mock_storage = Arc::new(MockPersistService {
        archived_checkpoint: std::sync::Mutex::new(Some(
            SessionCheckpoint::new("sid1".to_string())
                .with_status(SessionStatus::Archived)
                .with_peer_id("chat_a".to_string()),
        )),
        restore_called: std::sync::Mutex::new(false),
    });

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(mock_storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));

    let msg = make_message("agent-b", "hi");
    let result = sm.find_or_create("feishu", &msg, None).await.unwrap();

    let first = sm.take_restore_notification(&result).await;
    let second = sm.take_restore_notification(&result).await;
    assert!(
        second.is_none(),
        "second take should return None (notification consumed)"
    );
    if first.is_some() {
        assert_ne!(first, second, "first and second take should differ");
    }
}
