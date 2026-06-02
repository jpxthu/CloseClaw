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

use crate::gateway::{Gateway, GatewayConfig, HandleResult, SessionManager};
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::{Action, Defaults, Effect, Rule, RuleSet, Subject};
use crate::session::bootstrap::loader::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::slash::context::SlashContext;
use crate::slash::dispatcher::SlashDispatcher;
use crate::slash::handler::{SlashHandler, SlashResult};
use crate::slash::registry::HandlerRegistry;

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
    // → SlashHandled (message consumed) AND handler is NOT invoked.
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
        "non-owner + engine Deny must not invoke the handler"
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
async fn test_deny_does_not_invoke_handler() {
    // Stronger assertion of Branch 2: a mocked handler with an
    // `AtomicU32` counter MUST observe 0 calls when the engine denies.
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
        "Deny must consume the command and skip the handler"
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
