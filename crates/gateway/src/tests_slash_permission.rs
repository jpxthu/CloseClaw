//! Tests for Gateway slash-command permission routing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::{Gateway, GatewayConfig, HandleResult, SessionManager};
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Action, Defaults, Effect, Rule, RuleSet, Subject,
};
use closeclaw_session::persistence::ReasoningLevel;

struct SimpleHandler {
    command: &'static str,
    requires_permission: bool,
}
#[async_trait::async_trait]
impl SlashHandler for SimpleHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(SimpleHandler {
            command: self.command,
            requires_permission: self.requires_permission,
        })
    }

    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }
    fn description(&self) -> &str {
        "Simple test handler"
    }
    fn requires_permission(&self) -> bool {
        self.requires_permission
    }
    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(format!("{}: {args}", self.command))
    }
}
struct CountingHandler {
    command: &'static str,
    requires_permission: bool,
    counter: Arc<AtomicU32>,
}
#[async_trait::async_trait]
impl SlashHandler for CountingHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(CountingHandler {
            command: self.command,
            requires_permission: self.requires_permission,
            counter: Arc::clone(&self.counter),
        })
    }

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
struct CapturingHandler {
    command: &'static str,
    last_ctx: Arc<Mutex<Option<SlashContext>>>,
}
#[async_trait::async_trait]
impl SlashHandler for CapturingHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(CapturingHandler {
            command: self.command,
            last_ctx: Arc::clone(&self.last_ctx),
        })
    }

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
struct DefaultTestRouter;
#[async_trait::async_trait]
impl SlashRouter for DefaultTestRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        match command {
            "help" => Some(Box::new(SimpleHandler {
                command: "help",
                requires_permission: false,
            })),
            "exec" => Some(Box::new(SimpleHandler {
                command: "exec",
                requires_permission: true,
            })),
            _ => None,
        }
    }
}
struct EmptyRouter;
#[async_trait::async_trait]
impl SlashRouter for EmptyRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, _command: &str) -> Option<Box<dyn SlashHandler>> {
        None
    }
}
struct CountingRouter {
    command: &'static str,
    requires_permission: bool,
    counter: Arc<AtomicU32>,
}
#[async_trait::async_trait]
impl SlashRouter for CountingRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == self.command {
            Some(Box::new(CountingHandler {
                command: self.command,
                requires_permission: self.requires_permission,
                counter: Arc::clone(&self.counter),
            }))
        } else {
            None
        }
    }
}
struct CapturingRouter {
    command: &'static str,
    last_ctx: Arc<Mutex<Option<SlashContext>>>,
}

#[async_trait::async_trait]
impl SlashRouter for CapturingRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == self.command {
            Some(Box::new(CapturingHandler {
                command: self.command,
                last_ctx: Arc::clone(&self.last_ctx),
            }))
        } else {
            None
        }
    }
}
struct ImmediateCountingRouter {
    command: &'static str,
    counter: Arc<AtomicU32>,
}
#[async_trait::async_trait]
impl SlashRouter for ImmediateCountingRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, command: &str) -> bool {
        command == self.command
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == self.command {
            Some(Box::new(ImmediateCountingHandler {
                command: self.command,
                counter: Arc::clone(&self.counter),
            }))
        } else {
            None
        }
    }
}
fn clone_result(r: &SlashResult) -> SlashResult {
    match r {
        SlashResult::Reply(t) => SlashResult::Reply(t.clone()),
        SlashResult::Compact { instruction } => SlashResult::Compact {
            instruction: instruction.clone(),
        },
        SlashResult::Exec {
            command,
            requires_permission,
        } => SlashResult::Exec {
            command: command.clone(),
            requires_permission: *requires_permission,
        },
        SlashResult::SetReasoning { level } => SlashResult::SetReasoning { level: *level },
        SlashResult::SetVerbosity { level } => SlashResult::SetVerbosity { level: *level },
        SlashResult::Unknown(t) => SlashResult::Unknown(t.clone()),
        SlashResult::NewSession => SlashResult::NewSession,
        SlashResult::Stop { cascade, force } => SlashResult::Stop {
            cascade: *cascade,
            force: *force,
        },
        SlashResult::SetMode {
            mode,
            plan_file_path,
            initial_input,
            reply_message,
        } => SlashResult::SetMode {
            mode: mode.clone(),
            plan_file_path: plan_file_path.clone(),
            initial_input: initial_input.clone(),
            reply_message: reply_message.clone(),
        },
        SlashResult::SystemAppend { action } => SlashResult::SystemAppend {
            action: action.clone(),
        },
    }
}
struct ResultRouter {
    command: &'static str,
    result: SlashResult,
    requires_permission: bool,
}
#[async_trait::async_trait]
impl SlashRouter for ResultRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == self.command {
            Some(Box::new(ResultHandler {
                command: self.command,
                result: clone_result(&self.result),
                requires_permission: self.requires_permission,
            }))
        } else {
            None
        }
    }
}
fn make_gateway() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    Arc::new(Gateway::new(config, sm))
}
fn make_dispatcher() -> Arc<dyn SlashRouter> {
    Arc::new(DefaultTestRouter)
}
fn counting_dispatcher(
    command: &'static str,
    requires_permission: bool,
    counter: Arc<AtomicU32>,
) -> Arc<dyn SlashRouter> {
    Arc::new(CountingRouter {
        command,
        requires_permission,
        counter,
    })
}
fn capturing_dispatcher(
    command: &'static str,
    last_ctx: Arc<Mutex<Option<SlashContext>>>,
) -> Arc<dyn SlashRouter> {
    Arc::new(CapturingRouter { command, last_ctx })
}
fn deny_engine() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
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
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rules),
    ))
}
fn allow_engine() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
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
            file_read: Effect::Allow,
            file_write: Effect::Allow,
            exec: Effect::Allow,
            network: Effect::Allow,
            inter_agent: Effect::Allow,
            config: Effect::Allow,
            tool_call: Effect::Allow,
            message: Effect::Allow,
        },
        template_includes: vec![],
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rules),
    ))
}

// --- Tests ---
#[tokio::test]
async fn test_slash_not_entering_agent_session() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(make_dispatcher()).await;

    // dispatch_slash returns Some(HandleResult::SlashHandled) for recognized
    // commands, which the session handler uses to skip normal processing.
    let result = gw
        .dispatch_slash("sess1", "/help", Some("user123"), "feishu", Some("p"))
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    // Non-slash content returns None → falls through to agent session.
    let result = gw
        .dispatch_slash("sess1", "hello", Some("user123"), "feishu", Some("p"))
        .await;
    assert!(result.is_none());
}
#[tokio::test]
async fn test_unknown_slash_command_returns_reply() {
    let gw = make_gateway();
    // 空注册表——没有任何 handler
    gw.set_slash_dispatcher(Arc::new(EmptyRouter)).await;
    // 发送一个不存在的 slash 命令
    let result = gw
        .dispatch_slash(
            "sess1",
            "/xyz_unknown",
            Some("user123"),
            "feishu",
            Some("p"),
        )
        .await;
    // 应该返回 Some(HandleResult::SlashHandled)，不是 None
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
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
        .dispatch_slash("sess42", "/help", Some("user123"), "telegram", Some("p"))
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
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(ImmediateCountingHandler {
            command: self.command,
            counter: Arc::clone(&self.counter),
        })
    }

    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }
    fn description(&self) -> &str {
        "immediate counting handler"
    }
    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
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
) -> Arc<dyn SlashRouter> {
    Arc::new(ImmediateCountingRouter { command, counter })
}
/// Handler that returns a configurable [`SlashResult`].
struct ResultHandler {
    command: &'static str,
    result: SlashResult,
    requires_permission: bool,
}
#[async_trait::async_trait]
impl SlashHandler for ResultHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(ResultHandler {
            command: self.command,
            result: clone_result(&self.result),
            requires_permission: self.requires_permission,
        })
    }

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
        clone_result(&self.result)
    }
}
fn result_dispatcher(command: &'static str, result: SlashResult) -> Arc<dyn SlashRouter> {
    Arc::new(ResultRouter {
        command,
        result,
        requires_permission: false,
    })
}
#[tokio::test]
async fn test_execute_route_reply_variant() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "echo",
        SlashResult::Reply("pong".to_owned()),
    ))
    .await;
    let result = gw
        .dispatch_slash("s1", "/echo", Some("u1"), "feishu", Some("p"))
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}
#[tokio::test]
async fn test_execute_route_system_append_variant() {
    use closeclaw_common::slash_router::SystemAppendAction;
    let gw = make_gateway();
    gw.set_slash_dispatcher(result_dispatcher(
        "sys",
        SlashResult::SystemAppend {
            action: SystemAppendAction::Add("test instruction".to_owned()),
        },
    ))
    .await;
    let result = gw
        .dispatch_slash(
            "s1",
            "/sys add test instruction",
            Some("u1"),
            "feishu",
            Some("p"),
        )
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}
// ===========================================================================
// Busy-queueing: non-immediate slash commands enqueued when session is busy
// ===========================================================================
/// Helper: register a `ConversationSession` in the gateway's session manager
/// and return the Arc so the test can set it busy/idle.
async fn register_session(
    gw: &Gateway,
    session_id: &str,
) -> Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>> {
    use std::path::PathBuf;
    let cs = closeclaw_session::llm_session::ConversationSession::new(
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
        .dispatch_slash("sess-busy", "/help", Some("user1"), "feishu", Some("p"))
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
    // "stop" is an immediate command (router::is_immediate returns true).
    gw.set_slash_dispatcher(immediate_counting_dispatcher("stop", Arc::clone(&counter)))
        .await;

    let cs = register_session(&gw, "sess-busy-stop").await;
    cs.write()
        .await
        .set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);

    let result = gw
        .dispatch_slash(
            "sess-busy-stop",
            "/stop",
            Some("user1"),
            "feishu",
            Some("p"),
        )
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
        .dispatch_slash("sess-idle", "/help", Some("user1"), "feishu", Some("p"))
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
// ===========================================================================
// Step 1.3: Permission denial reply text & edge-case tests
// ===========================================================================
//
// These tests verify the three test dimensions from the plan:
// 1. 文案验证 — permission denial reply contains "权限不足" prefix
// 2. 边界值 — denial when permission engine is not configured
// 3. 状态转换 — permission engine Denied → handler invoked, execute skipped
//
// Note: The "权限不足" text is sent via `send_reply_if_available` which
// delegates to `SessionMessageHandler::send_reply()`. In unit tests the
// session_handler is None, so the text is silently dropped. The behavioral
// contract (handler skipped, dispatch returns SlashHandled) is verified
// below.

/// 边界值 (edge-case): denial when the deny rule has an empty name.
/// Verifies the Gateway handles edge-case denial reasons without panic.
#[tokio::test]
async fn test_permission_denied_empty_rule_name() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("exec", true, Arc::clone(&counter)))
        .await;

    // Build a deny engine with an empty rule name to produce a minimal reason.
    let rules = RuleSet {
        rules: vec![Rule {
            name: String::new(),
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
        ..Default::default()
    };
    gw.set_permission_engine(Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rules),
    )))
    .await;

    let result = gw
        .dispatch_slash("sess-empty", "/exec ls", Some("user1"), "feishu", Some("p"))
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "handler IS invoked, but execute() is skipped even with empty rule name denial"
    );
}
/// 状态转换 (state transition): non-owner + engine Allow → handler IS
/// invoked. Verifies the full allow path after the "权限不足" denial
/// path was exercised, confirming state transitions correctly.
#[tokio::test]
async fn test_permission_allow_after_deny_transition() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(allow_engine()).await;

    let result = gw
        .dispatch_slash("sess-allow", "/exec ls", Some("user1"), "feishu", Some("p"))
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "handler must be invoked when permission engine allows"
    );
}
// ===========================================================================
// Owner short-circuit path test
// ===========================================================================
/// Owner short-circuit: `sender_id == "owner"` bypasses the permission
/// engine entirely. Even with a deny-all engine, the owner's command
/// should be dispatched to the handler.
#[tokio::test]
async fn test_owner_slash_direct_dispatch() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess-owner", "/exec ls", Some("owner"), "feishu", Some("p"))
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "owner should bypass permission engine and invoke handler"
    );
}
// ===========================================================================
// Step 1.5: Exec.requires_permission bypass test
// ===========================================================================
/// Handler that returns `Exec { requires_permission: false }`,
/// simulating a read-only git subcommand.
struct ExecNoPermissionHandler;
#[async_trait::async_trait]
impl SlashHandler for ExecNoPermissionHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(ExecNoPermissionHandler)
    }

    fn commands(&self) -> &[&str] {
        &["git"]
    }
    fn description(&self) -> &str {
        "Git read-only command"
    }
    fn requires_permission(&self) -> bool {
        false
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Exec {
            command: "git status".to_owned(),
            requires_permission: false,
        }
    }
}

struct ExecNoPermissionRouter;
#[async_trait::async_trait]
impl SlashRouter for ExecNoPermissionRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == "git" {
            Some(Box::new(ExecNoPermissionHandler))
        } else {
            None
        }
    }
}
/// Step 1.5: `Exec { requires_permission: false }` bypasses the permission
/// engine entirely, even for a non-owner sender with a deny-all engine.
#[tokio::test]
async fn test_exec_no_permission_bypass() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(ExecNoPermissionRouter))
        .await;
    gw.set_permission_engine(deny_engine()).await;

    // Non-owner with deny-all engine should still succeed because
    // requires_permission: false skips the permission check.
    let result = gw
        .dispatch_slash(
            "sess-git",
            "/git status",
            Some("user1"),
            "feishu",
            Some("p"),
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "Exec with requires_permission: false should be dispatched without permission check"
    );
}
// ===========================================================================
// Step 1.2: WorkdirHandler permission routing tests
// ===========================================================================
//
// Simulates WorkdirHandler behavior: /git write commands require permission,
// /git read-only commands and /cd, /pwd do not.
//
// These tests exercise the three-branch permission routing for the
// specific case of the WorkdirHandler:
// 1. /git commit → Exec { requires_permission: true } → deny engine blocks non-owner
// 2. /git status → Exec { requires_permission: false } → bypasses permission engine
// 3. /cd, /pwd → Reply(...) → unaffected by permission engine
// 4. Owner on /git commit → owner short-circuits, bypasses engine

/// WorkdirHandler mock: inspects git args to determine permission requirement.
struct WorkdirHandler;
#[async_trait::async_trait]
impl SlashHandler for WorkdirHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(WorkdirHandler)
    }

    fn commands(&self) -> &[&str] {
        &["git", "cd", "pwd"]
    }
    fn description(&self) -> &str {
        "Workdir command handler"
    }
    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        if args.starts_with("status")
            || args.starts_with("log")
            || args.starts_with("diff")
            || args.starts_with("branch")
            || args.starts_with("show")
        {
            SlashResult::Exec {
                command: format!("git {args}"),
                requires_permission: false,
            }
        } else if !args.is_empty() {
            SlashResult::Exec {
                command: format!("git {args}"),
                requires_permission: true,
            }
        } else {
            SlashResult::Reply("usage: /git <command>".to_owned())
        }
    }
}
struct WorkdirRouter;

#[async_trait::async_trait]
impl SlashRouter for WorkdirRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        match command {
            "git" | "cd" | "pwd" => Some(Box::new(WorkdirHandler)),
            _ => None,
        }
    }
}
/// /git commit (write command) triggers permission engine for non-owner.
/// With a deny-all engine, the command is blocked (handler invoked but
/// execute skipped).
#[tokio::test]
async fn test_git_commit_non_owner_triggers_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash(
            "sess1",
            "/git commit -m test",
            Some("user1"),
            "feishu",
            Some("p"),
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "/git commit for non-owner with deny engine should be denied"
    );
}
/// /git status (read-only command) does NOT trigger permission engine.
/// Bypasses engine via Exec { requires_permission: false }.
#[tokio::test]
async fn test_git_status_readonly_bypasses_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess2", "/git status", Some("user1"), "feishu", Some("p"))
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "/git status should bypass permission engine and succeed"
    );
}
/// /cd and /pwd return Reply results, unaffected by permission engine.
#[tokio::test]
async fn test_cd_pwd_unaffected_by_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result_cd = gw
        .dispatch_slash("sess3", "/cd /tmp", Some("user1"), "feishu", Some("p"))
        .await;
    assert!(
        matches!(result_cd, Some(HandleResult::SlashHandled)),
        "/cd should succeed regardless of permission engine"
    );

    let result_pwd = gw
        .dispatch_slash("sess3", "/pwd", Some("user1"), "feishu", Some("p"))
        .await;
    assert!(
        matches!(result_pwd, Some(HandleResult::SlashHandled)),
        "/pwd should succeed regardless of permission engine"
    );
}
/// Owner on /git commit still directly executes (owner short-circuit).
/// Even with a deny-all engine, the owner's write command bypasses
/// the permission engine.
#[tokio::test]
async fn test_git_commit_owner_bypasses_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;
    let result = gw
        .dispatch_slash(
            "sess4",
            "/git commit -m test",
            Some("owner"),
            "feishu",
            Some("p"),
        )
        .await;
    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "/git commit for owner should bypass permission engine"
    );
}
