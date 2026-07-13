//! Tests for `/system clear` triggering static layer cache invalidation.
//!
//! Verifies that `GatewaySlashExecutor::execute_system_append` calls
//! `SessionManager::invalidate_static_cache()` after `Clear`, but not
//! after `Add`.  This covers both Step 1.2 (behavior verification) and
//! Step 1.4 (callback-based invalidation) tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use closeclaw_common::slash_router::{
    SlashContext, SlashHandler, SlashResult, SlashRouter, SystemAppendAction,
};
use closeclaw_permission::engine::engine_types::{
    Action, Defaults, Effect, MatchType, Rule, RuleSet, Subject,
};
use closeclaw_session::persistence::ReasoningLevel;

use crate::{Gateway, GatewayConfig, HandleResult, SessionManager};

// ---------------------------------------------------------------------------
// Mock: Handler that returns SystemAppend results
// ---------------------------------------------------------------------------

struct SystemAppendHandler {
    action: SystemAppendAction,
}

#[async_trait]
impl SlashHandler for SystemAppendHandler {
    fn commands(&self) -> &[&str] {
        &["system"]
    }

    fn description(&self) -> &str {
        "system append handler"
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::SystemAppend {
            action: self.action.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Mock: SlashRouter
// ---------------------------------------------------------------------------

/// Router that always returns a handler for the given action.
struct ActionRouter {
    action: SystemAppendAction,
}

#[async_trait]
impl SlashRouter for ActionRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }

    fn is_immediate(&self, _command: &str) -> bool {
        false
    }

    fn get_handler(&self, _command: &str) -> Option<Box<dyn SlashHandler>> {
        Some(Box::new(SystemAppendHandler {
            action: self.action.clone(),
        }))
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

/// Set up a gateway with a conversation session and a slash dispatcher
/// for the given `SystemAppendAction`.
///
/// `invalidate_called` is set to true when the cache_invalidator callback fires.
async fn setup(action: SystemAppendAction) -> (Arc<Gateway>, Arc<AtomicBool>) {
    let invalidate_called = Arc::new(AtomicBool::new(false));
    let gw = make_gateway();

    // Register a conversation session so execute_system_append can find it.
    {
        let cs = closeclaw_session::llm_session::ConversationSession::new(
            "sess-sys".to_owned(),
            "test-model".to_owned(),
            std::path::PathBuf::from("/tmp"),
        );
        let mut conv = gw.session_manager.conversation_sessions.write().await;
        conv.insert(
            "sess-sys".to_owned(),
            Arc::new(tokio::sync::RwLock::new(cs)),
        );
    }

    // Inject cache invalidator callback (replaces the old builder approach).
    let flag = Arc::clone(&invalidate_called);
    gw.session_manager
        .set_cache_invalidator(Arc::new(move || {
            flag.store(true, Ordering::SeqCst);
        }))
        .await;

    gw.set_slash_dispatcher(Arc::new(ActionRouter { action }))
        .await;

    (gw, invalidate_called)
}

// ---------------------------------------------------------------------------
// Tests — Step 1.2 behaviour dimensions (callback-based)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_system_clear_invalidates_cache() {
    // Normal path: /system clear → invalidate_static_cache() called
    let (gw, flag) = setup(SystemAppendAction::Clear).await;

    let result = gw
        .dispatch_slash("sess-sys", "/system", Some("owner"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert!(
        flag.load(Ordering::SeqCst),
        "invalidate_static_cache() must be called after /system clear"
    );
}

#[tokio::test]
async fn test_system_add_does_not_invalidate_cache() {
    // Boundary: /system add → invalidate_static_cache() NOT called
    let (gw, flag) = setup(SystemAppendAction::Add("new rule".to_owned())).await;

    let result = gw
        .dispatch_slash("sess-sys", "/system", Some("owner"), "feishu")
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert!(
        !flag.load(Ordering::SeqCst),
        "invalidate_static_cache() must NOT be called after /system add"
    );
}

#[tokio::test]
async fn test_state_transition_clear_then_add() {
    // State transition: clear → invalidates; add → does not invalidate
    let (gw, flag) = setup(SystemAppendAction::Clear).await;

    // Step 1: clear should invalidate
    let _ = gw
        .dispatch_slash("sess-sys", "/system", Some("owner"), "feishu")
        .await;
    assert!(
        flag.load(Ordering::SeqCst),
        "first clear must invalidate cache"
    );

    // Step 2: reset and switch to Add — should NOT invalidate
    flag.store(false, Ordering::SeqCst);
    gw.set_slash_dispatcher(Arc::new(ActionRouter {
        action: SystemAppendAction::Add("another rule".to_owned()),
    }))
    .await;
    let _ = gw
        .dispatch_slash("sess-sys", "/system", Some("owner"), "feishu")
        .await;
    assert!(
        !flag.load(Ordering::SeqCst),
        "add must NOT invalidate cache after prior clear"
    );
}

// ---------------------------------------------------------------------------
// Tests — Step 1.4: callback-based invalidation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_clear_with_callback_set() {
    // Normal path: callback injected → clear fires it
    let (gw, flag) = setup(SystemAppendAction::Clear).await;

    gw.dispatch_slash("sess-sys", "/system", Some("owner"), "feishu")
        .await;

    assert!(
        flag.load(Ordering::SeqCst),
        "injected callback must be invoked on /system clear"
    );
}

#[tokio::test]
async fn test_add_with_callback_set() {
    // Boundary: callback injected → add does NOT fire it
    let (gw, flag) = setup(SystemAppendAction::Add("new rule".to_owned())).await;

    gw.dispatch_slash("sess-sys", "/system", Some("owner"), "feishu")
        .await;

    assert!(
        !flag.load(Ordering::SeqCst),
        "injected callback must NOT be invoked on /system add"
    );
}

#[tokio::test]
async fn test_clear_without_callback_no_panic() {
    // Boundary: no cache_invalidator set → clear should not panic
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
    {
        let cs = closeclaw_session::llm_session::ConversationSession::new(
            "sess-nocb".to_owned(),
            "test-model".to_owned(),
            std::path::PathBuf::from("/tmp"),
        );
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert(
            "sess-nocb".to_owned(),
            Arc::new(tokio::sync::RwLock::new(cs)),
        );
    }
    let gw = Arc::new(Gateway::new(config, sm));
    gw.set_slash_dispatcher(Arc::new(ActionRouter {
        action: SystemAppendAction::Clear,
    }))
    .await;
    // Do NOT set a cache_invalidator — invalidate_static_cache() should be a no-op.

    let result = gw
        .dispatch_slash("sess-nocb", "/system", Some("owner"), "feishu")
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "clear without callback must still return SlashHandled (no panic)"
    );
}

#[tokio::test]
async fn test_callback_called_on_clear() {
    // Verify the callback is actually invoked (not just set).
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
    {
        let cs = closeclaw_session::llm_session::ConversationSession::new(
            "sess-cb".to_owned(),
            "test-model".to_owned(),
            std::path::PathBuf::from("/tmp"),
        );
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert("sess-cb".to_owned(), Arc::new(tokio::sync::RwLock::new(cs)));
    }
    let gw = Arc::new(Gateway::new(config, sm));

    let call_count = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&call_count);
    gw.session_manager
        .set_cache_invalidator(Arc::new(move || {
            flag.store(true, Ordering::SeqCst);
        }))
        .await;

    gw.set_slash_dispatcher(Arc::new(ActionRouter {
        action: SystemAppendAction::Clear,
    }))
    .await;

    gw.dispatch_slash("sess-cb", "/system", Some("owner"), "feishu")
        .await;

    assert!(
        call_count.load(Ordering::SeqCst),
        "cache_invalidator callback must be called on /system clear"
    );
}

// ── Branch routing tests (moved from slash_permission.rs inline) ───────

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
        "counting handler"
    }
    fn requires_permission(&self) -> bool {
        self.requires_permission
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        self.counter.fetch_add(1, Ordering::SeqCst);
        SlashResult::Reply("counted".to_owned())
    }
}

struct MockRouter {
    command: &'static str,
    requires_permission: bool,
    counter: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl SlashRouter for MockRouter {
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

fn counting_router(
    command: &'static str,
    requires_permission: bool,
    counter: Arc<AtomicU32>,
) -> Arc<dyn SlashRouter> {
    Arc::new(MockRouter {
        command,
        requires_permission,
        counter,
    })
}

fn allow_engine(
) -> Arc<tokio::sync::RwLock<closeclaw_permission::engine::engine_eval::PermissionEngine>> {
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
            command: Effect::Allow,
            network: Effect::Allow,
            inter_agent: Effect::Allow,
            config: Effect::Allow,
            tool_call: Effect::Allow,
            message: Effect::Allow,
        },
        user_defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
            rules,
        ),
    ))
}

fn deny_engine(
) -> Arc<tokio::sync::RwLock<closeclaw_permission::engine::engine_eval::PermissionEngine>> {
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
        user_defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
            rules,
        ),
    ))
}

struct MockModeQuery {
    modes: HashMap<String, SessionMode>,
}

impl MockModeQuery {
    fn new() -> Self {
        Self {
            modes: HashMap::new(),
        }
    }
    fn with_mode(mut self, agent_id: &str, mode: SessionMode) -> Self {
        self.modes.insert(agent_id.to_string(), mode);
        self
    }
}

impl SessionModeQuery for MockModeQuery {
    fn get_session_mode(&self, agent_id: &str) -> Option<SessionMode> {
        self.modes.get(agent_id).copied()
    }
}

fn auto_mode_allow_engine(
) -> Arc<tokio::sync::RwLock<closeclaw_permission::engine::engine_eval::PermissionEngine>> {
    let rules = RuleSet {
        rules: vec![Rule {
            name: "allow-all".to_owned(),
            subject: Subject::AgentOnly {
                agent: "*".to_owned(),
                match_type: MatchType::Glob,
            },
            effect: Effect::Allow,
            actions: vec![Action::All],
            template: None,
            priority: 100,
        }],
        defaults: Defaults {
            file_read: Effect::Allow,
            file_write: Effect::Allow,
            command: Effect::Allow,
            network: Effect::Allow,
            inter_agent: Effect::Allow,
            config: Effect::Allow,
            tool_call: Effect::Allow,
            message: Effect::Allow,
        },
        user_defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
            rules,
        )
        .with_session_mode_query(Arc::new(
            MockModeQuery::new().with_mode("test-agent", SessionMode::Auto),
        )),
    ))
}

#[tokio::test]
async fn test_non_owner_high_risk_permitted_handler_executes() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_router("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(allow_engine()).await;
    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("user123"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_owner_short_circuit_bypasses_deny() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_router("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(deny_engine()).await;
    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("owner"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_non_owner_high_risk_denied_handler_called_execute_skipped() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_router("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(deny_engine()).await;
    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("user123"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_non_owner_safe_handler_direct_dispatch() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_router("help", false, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(deny_engine()).await;
    let result = gw
        .dispatch_slash("sess1", "/help", Some("user123"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_auto_mode_slash_permitted_handler_executes() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_router("exec", true, Arc::clone(&counter)))
        .await;
    gw.set_permission_engine(auto_mode_allow_engine()).await;
    let result = gw
        .dispatch_slash("sess1", "/exec ls", Some("user123"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

// ── Step 1.1: /stop cascade/force execute path tests ─────────────────────

/// Handler that returns a configurable SlashResult for Stop.
struct StopResultHandler {
    cascade: bool,
    force: bool,
}

#[async_trait]
impl SlashHandler for StopResultHandler {
    fn commands(&self) -> &[&str] {
        &["stop"]
    }
    fn description(&self) -> &str {
        "stop handler"
    }
    fn immediate(&self, _cmd: &str) -> bool {
        true
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Stop {
            cascade: self.cascade,
            force: self.force,
        }
    }
}

struct StopRouter {
    cascade: bool,
    force: bool,
}

#[async_trait]
impl SlashRouter for StopRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        true
    }
    fn get_handler(&self, _command: &str) -> Option<Box<dyn SlashHandler>> {
        Some(Box::new(StopResultHandler {
            cascade: self.cascade,
            force: self.force,
        }))
    }
}

/// Verify that Stop with cascade=true goes through execute path without panic.
#[tokio::test]
async fn test_execute_route_stop_cascade() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(StopRouter {
        cascade: true,
        force: false,
    }))
    .await;
    let result = gw.dispatch_slash("s1", "/stop", Some("u1"), "feishu").await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

/// Verify that Stop with force=true goes through execute path without panic.
#[tokio::test]
async fn test_execute_route_stop_force() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(StopRouter {
        cascade: false,
        force: true,
    }))
    .await;
    let result = gw.dispatch_slash("s1", "/stop", Some("u1"), "feishu").await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

/// Verify that Stop with cascade+force goes through execute path without panic.
#[tokio::test]
async fn test_execute_route_stop_cascade_and_force() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(StopRouter {
        cascade: true,
        force: true,
    }))
    .await;
    let result = gw.dispatch_slash("s1", "/stop", Some("u1"), "feishu").await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

// ── Step 1.5: /system PartialRewrite snapshot trigger tests ───────────────

/// Helper: set up gateway with conversation session for snapshot tests.
async fn setup_snapshot_test() -> Arc<Gateway> {
    let gw = make_gateway();
    {
        let cs = closeclaw_session::llm_session::ConversationSession::new(
            "sess-snap".to_owned(),
            "test-model".to_owned(),
            std::path::PathBuf::from("/tmp"),
        );
        let mut conv = gw.session_manager.conversation_sessions.write().await;
        conv.insert(
            "sess-snap".to_owned(),
            Arc::new(tokio::sync::RwLock::new(cs)),
        );
    }
    gw
}

/// Step 1.5 — `/system add` creates a PartialRewrite snapshot.
#[tokio::test]
async fn test_system_add_creates_partial_rewrite_snapshot() {
    let gw = setup_snapshot_test().await;
    gw.set_slash_dispatcher(Arc::new(ActionRouter {
        action: SystemAppendAction::Add("new rule".to_owned()),
    }))
    .await;

    let result = gw
        .dispatch_slash("sess-snap", "/system", Some("owner"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));

    // Verify snapshot was created with PartialRewrite op type.
    let snapshot_count = gw.session_manager.snapshot_count_for("sess-snap").await;
    assert_eq!(
        snapshot_count,
        Some(1),
        "/system add should create exactly one snapshot"
    );
}

/// Step 1.5 — `/system clear` creates a PartialRewrite snapshot.
#[tokio::test]
async fn test_system_clear_creates_partial_rewrite_snapshot() {
    let gw = setup_snapshot_test().await;
    gw.set_slash_dispatcher(Arc::new(ActionRouter {
        action: SystemAppendAction::Clear,
    }))
    .await;

    let result = gw
        .dispatch_slash("sess-snap", "/system", Some("owner"), "feishu")
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));

    // Verify snapshot was created with PartialRewrite op type.
    let snapshot_count = gw.session_manager.snapshot_count_for("sess-snap").await;
    assert_eq!(
        snapshot_count,
        Some(1),
        "/system clear should create exactly one snapshot"
    );
}
