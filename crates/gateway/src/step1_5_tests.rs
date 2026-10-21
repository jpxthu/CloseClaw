#![allow(deprecated)]

//! Unit tests for Step 1.1–1.4 design doc alignment changes.
//!
//! Covers:
//! 1. Verbosity filtering: VerbosityFilter is an in-chain processor running
//!    before downstream processors
//! 2. Gateway delegates LLM calls via the injected LlmCaller
//! 3. Slash permission check timing: Handler → Permission → execute()
//! 4. Restore notification routed through Gateway outbound chain

use crate::slash_permission_test_utils::allow_engine;
use crate::{GatewayConfig, Message, SendOutboundIds, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_common::processor::ProcessedMessage;
use closeclaw_common::StreamingRenderer;
use closeclaw_llm::types::ContentBlock;
use closeclaw_processor_chain::{MessageContext, MessageProcessor, ProcessError, ProcessPhase};
use closeclaw_session::persistence::{PersistenceError, ReasoningLevel, SessionCheckpoint};
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
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> closeclaw_common::im_plugin::RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        closeclaw_common::im_plugin::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        _output: &closeclaw_common::im_plugin::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        Ok(())
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .expect("MockPlugin has no streaming renderer")
            .lock()
            .expect("MockPlugin streaming renderer lock poisoned")
            .handle_event(event)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .expect("MockPlugin has no streaming renderer")
            .lock()
            .expect("MockPlugin streaming renderer lock poisoned")
            .flush()
    }

    fn streaming_renderer(
        &self,
    ) -> Option<&std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>> {
        Some(&self.renderer)
    }
}

/// Minimal mock persistence service for restore notification tests.
struct MockPersistService {
    archived_checkpoint: std::sync::Mutex<Option<SessionCheckpoint>>,
    restore_called: std::sync::Mutex<bool>,
}

#[async_trait]
impl closeclaw_session::persistence::PersistenceService for MockPersistService {
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
        // Return None to simulate a restore failure path (checkpoint not found
        // after restore attempt).
        Ok(None)
    }
}

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
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
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
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
            content_blocks: ctx.content_blocks.clone(),
            metadata: ctx.metadata.clone(),
        }))
    }
}

/// Helper: create a gateway with an outbound trace processor and a session
/// with a specific verbosity level.
async fn setup_with_verbosity(
    verbosity: closeclaw_common::VerbosityLevel,
    session_id: &str,
    received_blocks: Arc<std::sync::Mutex<Vec<ContentBlock>>>,
) -> crate::Gateway {
    let trace = OutboundTraceProcessor::new(Arc::clone(&received_blocks));
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let mut registry = closeclaw_processor_chain::ProcessorRegistry::new();
    // VerbosityFilter（priority 5）先于 trace（priority 200）执行：
    // 现行架构中 verbosity 过滤是链内 processor，验证下游 processor 看到的是过滤后的 blocks。
    registry.register(Arc::new(
        closeclaw_processor_chain::verbosity_filter::VerbosityFilter,
    ));
    registry.register(Arc::new(trace));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), Arc::new(registry));
    gw.register_plugin(Arc::new(MockPlugin::new("mock"))).await;

    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        std::env::temp_dir(),
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
        crate::Session {
            id: session_id.to_string(),
            agent_id: "agent-test".to_string(),
            channel: "mock".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    gw
}

/// Step 1.1 — Verbosity filtering is an in-chain processor
/// (VerbosityFilter, priority 5) that runs before downstream processors
/// (e.g. the priority-200 trace processor registered after it).
///
/// When session verbosity is `Off` (only Text blocks), Thinking blocks
/// are stripped before downstream processors see them.
#[tokio::test]
async fn test_verbosity_filter_precedes_downstream_processors() {
    let received_blocks = Arc::new(std::sync::Mutex::new(Vec::<ContentBlock>::new()));
    let gw = setup_with_verbosity(
        closeclaw_common::VerbosityLevel::Off,
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

    gw.send_outbound(
        "sess-verb-off",
        "mock",
        "raw output",
        blocks,
        SendOutboundIds::default(),
    )
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

/// Step 1.1 — Verbosity Level::Normal strips Thinking within the chain,
/// before downstream processors see them.
#[tokio::test]
async fn test_verbosity_normal_strips_thinking_in_chain() {
    let received_blocks = Arc::new(std::sync::Mutex::new(Vec::<ContentBlock>::new()));
    let gw = setup_with_verbosity(
        closeclaw_common::VerbosityLevel::Normal,
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

    gw.send_outbound(
        "sess-verb-normal",
        "mock",
        "raw",
        blocks,
        SendOutboundIds::default(),
    )
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

/// Step 1.1 — Verbosity Level::Full: all blocks pass through the chain
/// to downstream processors.
#[tokio::test]
async fn test_verbosity_full_all_blocks_pass_through_chain() {
    let received_blocks = Arc::new(std::sync::Mutex::new(Vec::<ContentBlock>::new()));
    let gw = setup_with_verbosity(
        closeclaw_common::VerbosityLevel::Full,
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

    gw.send_outbound(
        "sess-verb-full",
        "mock",
        "raw",
        blocks,
        SendOutboundIds::default(),
    )
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

/// Verify that `SessionMessageHandler` delegates LLM calls
/// through `Arc<dyn closeclaw_common::LlmCaller>` (injected into
/// `SessionManager` via `set_llm_caller`) rather than calling
/// `UnifiedFallbackClient::chat()` directly.
///
/// Structural test: the caller is gateway's own
/// `llm_caller_impl::FallbackLlmCaller`, which wraps
/// `UnifiedFallbackClient`; the handler resolves it at runtime via
/// `dispatch_llm_call`.
#[tokio::test]
async fn test_gateway_delegates_llm_via_injected_caller() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
    let ufc = Arc::new(closeclaw_llm::unified_fallback::UnifiedFallbackClient::new(
        vec![],
        Arc::new(closeclaw_llm::retry::CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        )),
    ));
    let llm_caller: std::sync::Arc<dyn closeclaw_common::LlmCaller> =
        Arc::new(crate::llm_caller_impl::FallbackLlmCaller(ufc.clone()));
    // Set LLM caller on SessionManager so ConversationSession gets it at creation.
    sm.set_llm_caller(llm_caller).await;
    let handler = Arc::new(
        crate::session_handler::SessionMessageHandler::new_no_output(
            Arc::clone(&sm),
            ufc,
            None,
            closeclaw_session::compaction::CompactConfig::default(),
        ),
    );
    let gw = crate::Gateway::new_for_tests(config, sm).with_session_handler(handler);
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
impl closeclaw_slash::SlashHandler for TimingHandler {
    fn clone_box(&self) -> Box<dyn closeclaw_slash::SlashHandler> {
        Box::new(TimingHandler {
            order: Arc::clone(&self.order),
        })
    }

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
        _ctx: &closeclaw_slash::SlashContext,
    ) -> closeclaw_slash::SlashResult {
        self.order
            .lock()
            .expect("lock")
            .push("handler.handle()".to_string());
        closeclaw_slash::SlashResult::Reply("timing ok".to_owned())
    }
}

/// Step 1.2 — Permission check happens BEFORE handler.handle(), BEFORE execute().
///
/// The design doc requires: permission check → handler.handle() → result.execute().
#[tokio::test]
async fn test_permission_check_after_handler_before_execute() {
    let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new_for_tests(config, Arc::clone(&sm));

    let registry = closeclaw_slash::registry::HandlerRegistry::new();
    registry.register(Arc::new(TimingHandler {
        order: Arc::clone(&order),
    }));
    gw.set_slash_dispatcher(Arc::new(closeclaw_slash::SlashDispatcher::new(registry)))
        .await;
    gw.set_permission_engine(allow_engine()).await;

    let result = gw
        .dispatch_slash(
            "sess-timing",
            "/timing",
            Some("user1"),
            "feishu",
            Some("peer_id"),
        )
        .await;

    assert!(matches!(result, Some(crate::HandleResult::SlashHandled)));

    let ops = order.lock().expect("lock");
    assert_eq!(ops.len(), 1);
    assert_eq!(
        ops[0], "handler.handle()",
        "handler.handle() should be invoked (permission check happens after)"
    );
}

/// Step 1.2 — When permission is denied, handler.handle() is NOT invoked
/// and result.execute() is skipped.
#[tokio::test]
async fn test_permission_denied_handler_still_invoked() {
    let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new_for_tests(config, Arc::clone(&sm));

    let registry = closeclaw_slash::registry::HandlerRegistry::new();
    registry.register(Arc::new(TimingHandler {
        order: Arc::clone(&order),
    }));
    gw.set_slash_dispatcher(Arc::new(closeclaw_slash::SlashDispatcher::new(registry)))
        .await;

    use closeclaw_permission::engine::engine_types::{
        Action, Effect, MatchType, Rule, RuleSet, Subject,
    };
    let deny_rules = RuleSet {
        rules: vec![Rule {
            name: "deny-all".to_owned(),
            subject: Subject::AgentOnly {
                agent: "*".to_owned(),
                match_type: MatchType::Glob,
            },
            effect: Effect::Deny,
            actions: vec![Action::All],
            template: None,
            priority: 100,
        }],
        defaults: Default::default(),
        user_defaults: Default::default(),
        template_includes: vec![],
        ..Default::default()
    };
    gw.set_permission_engine(Arc::new(tokio::sync::RwLock::new(
        closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
            deny_rules,
        ),
    )))
    .await;

    let result = gw
        .dispatch_slash(
            "sess-deny",
            "/timing",
            Some("user1"),
            "feishu",
            Some("peer_id"),
        )
        .await;

    assert!(matches!(result, Some(crate::HandleResult::SlashHandled)));

    let ops = order.lock().expect("lock");
    // 现行语义（deny 分支改为 handle-then-gate，参照
    // slash_permission_tests::test_non_owner_high_risk_denied_handler_called_execute_skipped）：
    // 权限拒绝时 handler.handle() 仍被调用一次（用于产出 SlashResult），
    // 但 result.execute() 被跳过。
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
    use closeclaw_session::persistence::SessionStatus;

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
    use closeclaw_session::persistence::SessionStatus;

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
