//! Tests for `/system clear` triggering static layer cache invalidation.
//!
//! Verifies that `GatewaySlashExecutor::execute_system_append` calls
//! `SessionManager::invalidate_static_cache()` after `Clear`, but not
//! after `Add`.  This covers both Step 1.2 (behavior verification) and
//! Step 1.4 (callback-based invalidation) tests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::bootstrap::BootstrapMode;
use closeclaw_common::slash_router::{
    SlashContext, SlashHandler, SlashResult, SlashRouter, SystemAppendAction,
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

/// Set up a gateway with a conversation session and a slash dispatcher
/// for the given `SystemAppendAction`.
///
/// `invalidate_called` is set to true when the cache_invalidator callback fires.
async fn setup(action: SystemAppendAction) -> (Arc<Gateway>, Arc<AtomicBool>) {
    let invalidate_called = Arc::new(AtomicBool::new(false));
    let gw = make_gateway();

    // Register a conversation session so execute_system_append can find it.
    {
        let cs = closeclaw_llm::session::ConversationSession::new(
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
    {
        let cs = closeclaw_llm::session::ConversationSession::new(
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
    {
        let cs = closeclaw_llm::session::ConversationSession::new(
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
