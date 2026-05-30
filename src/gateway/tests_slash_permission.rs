//! Integration tests for Gateway slash-command permission routing.

use std::collections::HashMap;
use std::sync::Arc;

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

struct SafeHandler;

#[async_trait::async_trait]
impl SlashHandler for SafeHandler {
    fn name(&self) -> &str {
        "help"
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply("help!".to_owned())
    }
}

struct RiskyHandler;

#[async_trait::async_trait]
impl SlashHandler for RiskyHandler {
    fn name(&self) -> &str {
        "exec"
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply(format!("exec: {args}"))
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
    let mut registry = HandlerRegistry::new();
    registry.register(Arc::new(SafeHandler));
    registry.register(Arc::new(RiskyHandler));
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
    let gw = make_gateway();
    gw.set_slash_dispatcher(make_dispatcher()).await;
    gw.set_permission_engine(deny_engine()).await;

    // Even though the engine denies everything, owner bypasses it.
    let result = gw.dispatch_slash("sess1", "/exec ls", Some("owner")).await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_non_owner_high_risk_goes_to_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(make_dispatcher()).await;
    gw.set_permission_engine(deny_engine()).await;

    // Non-owner + requires_permission=true → engine denies → SlashHandled
    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("user123"))
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_non_owner_normal_slash_direct_dispatch() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(make_dispatcher()).await;
    // Engine denies everything, but "help" doesn't require permission
    gw.set_permission_engine(deny_engine()).await;

    let result = gw.dispatch_slash("sess1", "/help", Some("user123")).await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

#[tokio::test]
async fn test_slash_not_entering_agent_session() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(make_dispatcher()).await;

    // dispatch_slash returns Some(HandleResult::SlashHandled) for recognized
    // commands, which the session handler uses to skip normal processing.
    let result = gw.dispatch_slash("sess1", "/help", Some("user123")).await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));

    // Non-slash content returns None → falls through to agent session.
    let result = gw.dispatch_slash("sess1", "hello", Some("user123")).await;
    assert!(result.is_none());
}
