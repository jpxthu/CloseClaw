//! Tests for ClearHandler cache invalidation (Step 1.1 / Step 1.3).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handlers::ClearHandler;
use closeclaw_common::slash_router::{SlashHandler, SlashResult};
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_session::persistence::ReasoningLevel;

fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

fn make_sm() -> Arc<SessionManager> {
    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &gc,
        None,
        None,
        ReasoningLevel::default(),
    ))
}

/// Verify that /clear fires the cache_invalidator callback,
/// which is responsible for clearing the shared SectionCache.
#[tokio::test]
async fn test_clear_fires_cache_invalidator_callback() {
    let sm = make_sm();

    let invalidator_called = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&invalidator_called);
    sm.set_cache_invalidator(Arc::new(move || {
        flag.store(true, Ordering::SeqCst);
    }))
    .await;

    let handler = ClearHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = dummy_ctx();
    let result = handler.handle("", &ctx).await;

    match result {
        SlashResult::Reply(text) => {
            assert!(text.contains("System prompt 缓存已清除"), "got: {text}");
        }
        _ => panic!("expected Reply variant"),
    }

    assert!(
        invalidator_called.load(Ordering::SeqCst),
        "cache_invalidator must be called by /clear"
    );
}

/// Verify call order: `invalidate_static_cache()` is called before
/// `rebuild_system_prompt_for_session()`. Both complete successfully.
#[tokio::test]
async fn test_clear_calls_invalidate_before_rebuild() {
    let sm = make_sm();

    let invalidate_fired = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&invalidate_fired);
    sm.set_cache_invalidator(Arc::new(move || {
        flag.store(true, Ordering::SeqCst);
    }))
    .await;

    let handler = ClearHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = dummy_ctx();
    let result = handler.handle("", &ctx).await;

    // handle() returns Reply → both invalidate and rebuild completed.
    assert!(matches!(result, SlashResult::Reply(_)));
    assert!(
        invalidate_fired.load(Ordering::SeqCst),
        "invalidate_static_cache must be called during /clear"
    );
}

/// /clear without cache_invalidator set should not panic.
#[tokio::test]
async fn test_clear_without_cache_invalidator_no_panic() {
    let sm = make_sm();
    // Do NOT set cache_invalidator — should be a no-op.

    let handler = ClearHandler::new(sm as Arc<dyn closeclaw_common::SlashSessionQuery>);
    let ctx = dummy_ctx();
    let result = handler.handle("", &ctx).await;

    assert!(
        matches!(result, SlashResult::Reply(_)),
        "/clear without cache_invalidator should still return Reply"
    );
}
