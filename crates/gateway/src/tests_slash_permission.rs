//! Integration tests for Gateway slash-command permission routing.
//!
//! Covers the three-branch permission routing introduced in Step 1.2:
//! 1. Owner short-circuit — owner bypasses permission engine for any command.
//! 2. Non-owner + `requires_permission() == true` — routed through the engine;
//!    `Denied` consumes the command (handler is NOT invoked) and replies
//!    "无权限".
//! 3. Non-owner + `requires_permission() == false` — directly dispatched.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::{Gateway, GatewayConfig, HandleResult, SessionManager};
use closeclaw_common::slash_router::context::SlashContext;
use closeclaw_common::slash_router::dispatcher::SlashDispatcher;
use closeclaw_common::slash_router::handler::{SlashHandler, SlashResult};
use closeclaw_common::slash_router::registry::HandlerRegistry;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Action, Defaults, Effect, Rule, RuleSet, Subject,
};
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;

// ---------------------------------------------------------------------------
// Mock handlers
// ---------------------------------------------------------------------------

/// Safe handler — does NOT require permission.
struct SafeHandler;

#[async_trait::async_trait]
impl SlashHandler for SafeHandler {
    fn commands(&self) -> &[&str] {
        &["help"]
    }

    fn description(&self) -> &str {
        "Help command"
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply("help!".to_owned())
    }
}

/// Risky handler — overrides `requires_permission()` to `true`.
///
/// `SlashHandler::requires_permission()` has a default of `false`; this
/// override is what makes `/exec` take Branch 2 (engine path) in
/// `dispatch_slash`.
struct RiskyHandler;

#[async_trait::async_trait]
impl SlashHandler for RiskyHandler {
    fn commands(&self) -> &[&str] {
        &["exec"]
    }

    fn description(&self) -> &str {
        "Execute a shell command"
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(format!("exec: {args}"))
    }
}

/// Handler that records how many times `handle()` was invoked.
///
/// Used to assert that `dispatch_slash` either invokes or skips a handler
/// based on the permission routing branch.
struct CountingHandler {
    command: &'static str,
    requires_permission: bool,
    counter: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl SlashHandler for CountingHandler {
    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }

    fn description(&self) -> &str {
        "Counting handler"
    }

    fn requires_permission(&self) -> bool {
        self.requires_permission
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        self.counter.fetch_add(1, Ordering::SeqCst);
        SlashResult::Reply("counted".to_owned())
    }
}

/// Handler that captures the most recent `SlashContext` it was invoked with.
///
/// Used to verify that `dispatch_slash` populates `SlashContext.channel`
/// with the `channel` argument from the call site.
struct CapturingHandler {
    command: &'static str,
    last_ctx: Arc<Mutex<Option<SlashContext>>>,
}

#[async_trait::async_trait]
impl SlashHandler for CapturingHandler {
    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }

    fn description(&self) -> &str {
        "Capturing handler"
    }

    async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
        *self.last_ctx.lock().expect("ctx mutex poisoned") = Some(SlashContext {
            command: ctx.command.clone(),
            sender_id: ctx.sender_id.clone(),
            session_id: ctx.session_id.clone(),
            channel: ctx.channel.clone(),
        });
        SlashResult::Reply("captured".to_owned())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_gateway() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: Default::default(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    Arc::new(Gateway::new(config, sm))
}

fn make_dispatcher() -> Arc<SlashDispatcher> {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(SafeHandler));
    registry.register(Arc::new(RiskyHandler));
    Arc::new(SlashDispatcher::new(registry))
}

/// Build a dispatcher that contains a `CountingHandler` for a given command.
fn counting_dispatcher(
    command: &'static str,
    requires_permission: bool,
    counter: Arc<AtomicU32>,
) -> Arc<SlashDispatcher> {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(CountingHandler {
        command,
        requires_permission,
        counter,
    }));
    Arc::new(SlashDispatcher::new(registry))
}

/// Build a dispatcher that contains a `CapturingHandler` for a given command.
fn capturing_dispatcher(
    command: &'static str,
    last_ctx: Arc<Mutex<Option<SlashContext>>>,
) -> Arc<SlashDispatcher> {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(CapturingHandler { command, last_ctx }));
    Arc::new(SlashDispatcher::new(registry))
}

/// A PermissionEngine that always denies.
fn deny_engine() -> Arc<PermissionEngine> {
    let rules = RuleSet {
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
        defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
    };
    Arc::new(PermissionEngine::new_with_default_data_root(rules))
}

/// A PermissionEngine that always allows (all rules are Allow).
fn allow_engine() -> Arc<PermissionEngine> {
    let rules = RuleSet {
        rules: vec![Rule {
            name: "allow-all".to_owned(),
            subject: Subject::AgentOnly {
                agent: "*".to_owned(),
                match_type: Default::default(),
            },
            effect: Effect::Allow,
            actions: vec![Action::All],
            template: None,
            priority: 100,
        }],
        defaults: Defaults {
            file: Effect::Allow,
            command: Effect::Allow,
            network: Effect::Allow,
            inter_agent: Effect::Allow,
            config: Effect::Allow,
            tool_call: Effect::Allow,
        },
        template_includes: vec![],
        agent_creators: HashMap::new(),
    };
    Arc::new(PermissionEngine::new_with_default_data_root(rules))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_owner_slash_direct_dispatch() {
    // Branch 1: owner short-circuits the engine. Even with `requires_permission`
    // handler and a deny-all engine, the handler MUST be invoked.
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("owner"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "owner must bypass the deny-all engine and invoke the handler"
    );
}

#[tokio::test]
async fn test_non_owner_high_risk_goes_to_permission_engine() {
    // Branch 2: non-owner + requires_permission=true + engine Deny
    // → handler.handle() IS invoked (returns SlashResult), but execute() is
    //   skipped because permission check denies after handler.
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("user123"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "handler.handle() must NOT be invoked when permission is denied"
    );
}

#[tokio::test]
async fn test_non_owner_normal_slash_direct_dispatch() {
    // Branch 3: non-owner + requires_permission=false → handler invoked,
    // engine is never consulted.
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("help", false, Arc::clone(&counter)))
        .await;
    // Install a deny engine: if dispatch_slash ever consults the engine for
    // a non-permissioned command, the test would observe an unexpected path.
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess1", "/help", Some("user123"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "non-owner + safe handler must dispatch directly without consulting the engine"
    );
}

#[tokio::test]
async fn test_slash_not_entering_agent_session() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(make_dispatcher()).await;

    // dispatch_slash returns Some(HandleResult::SlashHandled) for recognized
    // commands, which the session handler uses to skip normal processing.
    let result = gw
        .dispatch_slash("sess1", "/help", Some("user123"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));

    // Non-slash content returns None → falls through to agent session.
    let result = gw
        .dispatch_slash("sess1", "hello", Some("user123"), "feishu")
        .await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_unknown_slash_command_returns_reply() {
    let gw = make_gateway();
    // 空注册表——没有任何 handler
    gw.set_slash_dispatcher(Arc::new(SlashDispatcher::new(HandlerRegistry::new())))
        .await;

    // 发送一个不存在的 slash 命令
    let result = gw
        .dispatch_slash("sess1", "/xyz_unknown", Some("user123"), "feishu")
        .await;
    // 应该返回 Some(HandleResult::SlashHandled)，不是 None
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_deny_skips_execute() {
    // Branch 2: handler.handle() IS invoked, but permission check denies
    // after handler returns, so result.execute() is skipped.
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess1", "/exec rm -rf /", Some("user123"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "handler.handle() must NOT be invoked when permission is denied"
    );
}

#[tokio::test]
async fn test_non_owner_high_risk_permitted_handler_executes() {
    // Branch 2: non-owner + requires_permission=true + engine Allow
    // → handler.handle() IS invoked and result.execute() runs.
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(allow_engine()).await;

    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("user123"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "handler.handle() must be invoked when permission is allowed"
    );
}

#[tokio::test]
async fn test_slash_context_channel_propagates() {
    // `dispatch_slash`'s `channel` argument must be visible to the handler
    // via `SlashContext.channel`.
    let last_ctx: Arc<Mutex<Option<SlashContext>>> = Arc::new(Mutex::new(None));
    let gw = make_gateway();
    gw.set_slash_dispatcher(capturing_dispatcher("help", Arc::clone(&last_ctx)))
        .await;

    let result = gw
        .dispatch_slash("sess42", "/help", Some("user123"), "telegram")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    let guard = last_ctx.lock().expect("ctx mutex poisoned");
    let captured = guard.as_ref().expect("handler was not invoked");
    assert_eq!(captured.channel, "telegram");
    assert_eq!(captured.session_id, "sess42");
    assert_eq!(captured.sender_id, "user123");
}

// ===========================================================================
// execute_and_route: SlashResult.execute() path tests
// ===========================================================================
//
// These tests verify that `dispatch_slash` → `execute_and_route` correctly
// routes every `SlashResult` variant through the new `SideEffectContext::
// execute()` path. Each handler returns a specific variant, and we assert
// that `dispatch_slash` returns `SlashHandled` (meaning the execute path
// ran without panic).

/// Handler that claims to be immediate (responds even when LLM is busy).
struct ImmediateCountingHandler {
    command: &'static str,
    counter: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl SlashHandler for ImmediateCountingHandler {
    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }
    fn description(&self) -> &str {
        "immediate counting handler"
    }
    fn immediate(&self, _cmd: &str) -> bool {
        true
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        self.counter.fetch_add(1, Ordering::SeqCst);
        SlashResult::Reply("counted".to_owned())
    }
}

/// Build a dispatcher that contains an `ImmediateCountingHandler` for a given command.
fn immediate_counting_dispatcher(
    command: &'static str,
    counter: Arc<AtomicU32>,
) -> Arc<SlashDispatcher> {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(ImmediateCountingHandler { command, counter }));
    Arc::new(SlashDispatcher::new(registry))
}

/// Handler that returns a configurable [`SlashResult`].
struct ResultHandler {
    command: &'static str,
    result: SlashResult,
    requires_permission: bool,
}

#[async_trait::async_trait]
impl SlashHandler for ResultHandler {
    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }
    fn description(&self) -> &str {
        "result handler"
    }
    fn requires_permission(&self) -> bool {
        self.requires_permission
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        // Clone so each invocation returns the same variant.
        match &self.result {
            SlashResult::Reply(t) => SlashResult::Reply(t.clone()),
            SlashResult::Compact { instruction } => SlashResult::Compact {
                instruction: instruction.clone(),
            },
            SlashResult::Exec { command } => SlashResult::Exec {
                command: command.clone(),
            },
            SlashResult::SetReasoning { level } => SlashResult::SetReasoning { level: *level },
            SlashResult::SetVerbosity { level } => SlashResult::SetVerbosity { level: *level },
            SlashResult::Unknown(t) => SlashResult::Unknown(t.clone()),
            SlashResult::NewSession => SlashResult::NewSession,
            SlashResult::Stop => SlashResult::Stop,
            SlashResult::SetMode(t) => SlashResult::SetMode(t.clone()),
            SlashResult::SystemAppend { action } => SlashResult::SystemAppend {
                action: action.clone(),
            },
        }
    }
}

fn result_dispatcher(command: &'static str, result: SlashResult) -> Arc<SlashDispatcher> {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(ResultHandler {
        command,
        result,
        requires_permission: false,
    }));
    Arc::new(SlashDispatcher::new(registry))
}

#[tokio::test]
async fn test_execute_route_reply_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "echo",
        SlashResult::Reply("pong".to_owned()),
    ))
    .await;
    let result = gw.dispatch_slash("s1", "/echo", Some("u1"), "feishu").await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_compact_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "compact",
        SlashResult::Compact {
            instruction: Some("keep summary".to_owned()),
        },
    ))
    .await;
    let result = gw
        .dispatch_slash("s1", "/compact keep summary", Some("u1"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_exec_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "exec",
        SlashResult::Exec {
            command: "ls".to_owned(),
        },
    ))
    .await;
    let result = gw
        .dispatch_slash("s1", "/exec ls", Some("u1"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_unknown_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "unk",
        SlashResult::Unknown("not found".to_owned()),
    ))
    .await;
    let result = gw.dispatch_slash("s1", "/unk", Some("u1"), "feishu").await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_set_reasoning_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "reasoning",
        SlashResult::SetReasoning {
            level: closeclaw_session::persistence::ReasoningLevel::High,
        },
    ))
    .await;
    let result = gw
        .dispatch_slash("s1", "/reasoning high", Some("u1"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_set_verbosity_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "verbose",
        SlashResult::SetVerbosity {
            level: closeclaw_common::VerbosityLevel::Off,
        },
    ))
    .await;
    let result = gw
        .dispatch_slash("s1", "/verbose off", Some("u1"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_new_session_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher("new", SlashResult::NewSession))
        .await;
    let result = gw.dispatch_slash("s1", "/new", Some("u1"), "feishu").await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_stop_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher("stop", SlashResult::Stop))
        .await;
    let result = gw.dispatch_slash("s1", "/stop", Some("u1"), "feishu").await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_system_append_variant() {
    use closeclaw_common::slash_router::handler::SystemAppendAction;
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "sys",
        SlashResult::SystemAppend {
            action: SystemAppendAction::Add("test instruction".to_owned()),
        },
    ))
    .await;
    let result = gw
        .dispatch_slash("s1", "/sys add test instruction", Some("u1"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_all_variants_return_slash_handled() {
    // Comprehensive: all recognized variants go through execute_and_route
    // and return SlashHandled.
    let gw = make_gateway();
    let cases: Vec<(&str, SlashResult)> = vec![
        ("a", SlashResult::Reply("r".to_owned())),
        ("b", SlashResult::Compact { instruction: None }),
        (
            "c",
            SlashResult::Exec {
                command: "x".to_owned(),
            },
        ),
        (
            "d",
            SlashResult::SetReasoning {
                level: closeclaw_session::persistence::ReasoningLevel::Low,
            },
        ),
        (
            "e",
            SlashResult::SetVerbosity {
                level: closeclaw_common::VerbosityLevel::Normal,
            },
        ),
        ("f", SlashResult::Unknown("?".to_owned())),
        ("g", SlashResult::NewSession),
        ("h", SlashResult::Stop),
        ("i", SlashResult::SetMode("dark".to_owned())),
    ];
    for (cmd, result) in cases {
        gw.set_slash_dispatcher(result_dispatcher(cmd, result))
            .await;
        let dispatch_result = gw
            .dispatch_slash("s1", &format!("/{cmd}"), Some("u1"), "feishu")
            .await;
        assert!(
            matches!(dispatch_result, Some(HandleResult::SlashHandled)),
            "variant /{cmd} should return SlashHandled"
        );
    }
}

#[tokio::test]
async fn test_execute_route_permission_check_before_execute() {
    // Owner bypasses permission engine AND goes through execute_and_route.
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "exec",
        SlashResult::Exec {
            command: "ls".to_owned(),
        },
    ))
    .await;
    gw.set_permission_engine(deny_engine()).await;

    // Owner must bypass the deny engine and still reach execute_and_route.
    let result = gw
        .dispatch_slash("s1", "/exec ls", Some("owner"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_execute_route_non_owner_denied_skips_execute() {
    // Non-owner with requires_permission=true and deny engine
    // → handler.handle() IS invoked, but permission check denies
    //   after handler, so result.execute() is skipped.
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "exec",
        SlashResult::Exec {
            command: "ls".to_owned(),
        },
    ))
    .await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("s1", "/exec ls", Some("user1"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    // handler.handle() IS invoked, but result.execute() is skipped because permission is checked after handler returns SlashResult.
}

// ===========================================================================
// Busy-queueing: non-immediate slash commands enqueued when session is busy
// ===========================================================================

/// Helper: register a `ConversationSession` in the gateway's session manager
/// and return the Arc so the test can set it busy/idle.
async fn register_session(
    gw: &Gateway,
    session_id: &str,
) -> Arc<tokio::sync::RwLock<closeclaw_llm::session::ConversationSession>> {
    use std::path::PathBuf;
    let cs = closeclaw_llm::session::ConversationSession::new(
        session_id.to_owned(),
        "test-model".to_owned(),
        PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        let mut conv = gw.session_manager.conversation_sessions.write().await;
        conv.insert(session_id.to_owned(), cs_arc.clone());
    }
    cs_arc
}

#[tokio::test]
async fn test_non_immediate_busy_enqueues_and_returns_slash_handled() {
    // Non-immediate command + session busy → enqueued, handler NOT invoked,
    // returns SlashHandled.
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("help", false, Arc::clone(&counter)))
        .await;

    let cs = register_session(&gw, "sess-busy").await;
    cs.write()
        .await
        .set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);

    let result = gw
        .dispatch_slash("sess-busy", "/help", Some("user1"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "handler must NOT be invoked when session is busy"
    );

    // Verify the command was enqueued as a pending message.
    let cs = gw
        .session_manager
        .get_conversation_session("sess-busy")
        .await
        .unwrap();
    let cs = cs.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(
        pending.len(),
        1,
        "exactly one pending message should be enqueued"
    );
    assert!(
        pending[0].content.contains("/help"),
        "pending message should contain the slash command"
    );
}

#[tokio::test]
async fn test_immediate_busy_executes_normally() {
    // Immediate command + session busy → handler IS invoked (no enqueue).
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    // "stop" is an immediate command (SlashDispatcher::is_immediate returns true).
    gw.set_slash_dispatcher(immediate_counting_dispatcher("stop", Arc::clone(&counter)))
        .await;

    let cs = register_session(&gw, "sess-busy-stop").await;
    cs.write()
        .await
        .set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);

    let result = gw
        .dispatch_slash("sess-busy-stop", "/stop", Some("user1"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "immediate handler must be invoked even when busy"
    );

    // No pending messages should be enqueued.
    let cs = gw
        .session_manager
        .get_conversation_session("sess-busy-stop")
        .await
        .unwrap();
    let cs = cs.read().await;
    assert_eq!(
        cs.get_pending_messages().len(),
        0,
        "immediate command must NOT be enqueued"
    );
}

#[tokio::test]
async fn test_non_immediate_idle_executes_normally() {
    // Non-immediate command + session idle → handler IS invoked (no enqueue).
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("help", false, Arc::clone(&counter)))
        .await;

    let cs = register_session(&gw, "sess-idle").await;
    cs.write()
        .await
        .set_llm_state(closeclaw_llm::session_state::LlmState::Idle);

    let result = gw
        .dispatch_slash("sess-idle", "/help", Some("user1"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "handler must be invoked when session is idle"
    );

    // No pending messages should be enqueued.
    let cs = gw
        .session_manager
        .get_conversation_session("sess-idle")
        .await
        .unwrap();
    let cs = cs.read().await;
    assert_eq!(
        cs.get_pending_messages().len(),
        0,
        "idle session must NOT enqueue"
    );
}
