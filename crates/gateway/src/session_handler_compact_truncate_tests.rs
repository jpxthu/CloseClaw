#![allow(deprecated)]

//! Unit tests for auto-compact history truncation (Step 1.3).
//!
//! Verifies that `check_and_run_auto_compact` truncates the persistent
//! transcript to `max_history_messages` before estimating tokens, and
//! that the session's history is consistent with what `load_compact_inputs`
//! returns after truncation (single source of truth).

use super::*;
use crate::session_handler::ActiveSearcherLlmCaller;
use closeclaw_common::ContentBlock;
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::ChatSession;
use closeclaw_llm::LLMRegistry;
use closeclaw_session::compaction::CompactConfig;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_config() -> crate::GatewayConfig {
    crate::GatewayConfig {
        name: "compact_truncate_test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_sm() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &make_config(),
        None,
        None,
        ReasoningLevel::default(),
    ))
}

fn make_fallback_client() -> Arc<FallbackClient> {
    Arc::new(FallbackClient::from_strings(
        Arc::new(LLMRegistry::new()),
        vec![],
    ))
}

fn make_active_searcher_caller() -> Arc<ActiveSearcherLlmCaller> {
    Arc::new(ActiveSearcherLlmCaller {
        client: Arc::new(UnifiedFallbackClient::new(
            vec![],
            Arc::new(CooldownManager::new()),
        )),
        model: String::new(),
    })
}

/// Create a handler with no output channel and the given compact config.
fn handler_no_output(sm: &Arc<SessionManager>, config: CompactConfig) -> SessionMessageHandler {
    SessionMessageHandler::new_no_output(
        Arc::clone(sm),
        make_fallback_client(),
        make_active_searcher_caller(),
        config,
    )
}

/// Insert a ConversationSession with `n` pre-populated messages.
async fn insert_session_with_messages(sm: &SessionManager, session_id: &str, n: usize) {
    let cs = ConversationSession::new(
        session_id.to_string(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );
    let mut cs_write = cs;
    for i in 0..n {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        cs_write.append_transcript(role, vec![ContentBlock::Text(format!("msg{i}"))]);
    }
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs_write));
    sm.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), cs_arc);
}

// ═════════════════════════════════════════════════════════════════════════════
// Error path: session not found → silent return
// ═════════════════════════════════════════════════════════════════════════════

/// When the session does not exist, `check_and_run_auto_compact` returns
/// silently without panicking or emitting errors.
#[tokio::test]
async fn test_auto_compact_nonexistent_session_returns_silently() {
    let sm = make_sm();
    let config = CompactConfig {
        max_history_messages: Some(10),
        ..Default::default()
    };
    let handler = handler_no_output(&sm, config);
    // Should not panic.
    handler.check_and_run_auto_compact("nonexistent").await;
}

/// When `max_history_messages` is None, truncation is skipped entirely
/// even if the session exists with many messages.
#[tokio::test]
async fn test_auto_compact_none_max_skips_truncation() {
    let sm = make_sm();
    insert_session_with_messages(&sm, "s-none", 20).await;
    let config = CompactConfig {
        max_history_messages: None,
        ..Default::default()
    };
    let handler = handler_no_output(&sm, config);
    handler.check_and_run_auto_compact("s-none").await;
    // Messages should remain unchanged.
    let cs = sm.get_conversation_session("s-none").await.unwrap();
    let cs_read = cs.read().await;
    assert_eq!(cs_read.messages().len(), 20);
}

// ═════════════════════════════════════════════════════════════════════════════
// Integration: truncation makes persistent history = token estimation input
// ═════════════════════════════════════════════════════════════════════════════

/// After `check_and_run_auto_compact` runs with `max_history_messages` set,
/// the session's persistent history should be truncated to at most `max`
/// messages. We verify by reading the session directly after the call.
#[tokio::test]
async fn test_auto_compact_truncates_persistent_history() {
    let sm = make_sm();
    insert_session_with_messages(&sm, "s-trunc", 15).await;
    let config = CompactConfig {
        max_history_messages: Some(5),
        chars_per_token: 0.25,
        ..Default::default()
    };
    let handler = handler_no_output(&sm, config);
    handler.check_and_run_auto_compact("s-trunc").await;
    // Persistent history should be truncated to 5 messages.
    let cs = sm.get_conversation_session("s-trunc").await.unwrap();
    let cs_read = cs.read().await;
    assert_eq!(
        cs_read.messages().len(),
        5,
        "persistent history should be truncated to max_history_messages"
    );
}

/// Session with fewer messages than max → no truncation occurs.
#[tokio::test]
async fn test_auto_compact_no_truncation_when_below_limit() {
    let sm = make_sm();
    insert_session_with_messages(&sm, "s-below", 3).await;
    let config = CompactConfig {
        max_history_messages: Some(10),
        chars_per_token: 0.25,
        ..Default::default()
    };
    let handler = handler_no_output(&sm, config);
    handler.check_and_run_auto_compact("s-below").await;
    let cs = sm.get_conversation_session("s-below").await.unwrap();
    let cs_read = cs.read().await;
    assert_eq!(
        cs_read.messages().len(),
        3,
        "messages should remain unchanged when below limit"
    );
}

/// Session with exactly max messages → no truncation.
#[tokio::test]
async fn test_auto_compact_no_truncation_when_at_limit() {
    let sm = make_sm();
    insert_session_with_messages(&sm, "s-exact", 5).await;
    let config = CompactConfig {
        max_history_messages: Some(5),
        chars_per_token: 0.25,
        ..Default::default()
    };
    let handler = handler_no_output(&sm, config);
    handler.check_and_run_auto_compact("s-exact").await;
    let cs = sm.get_conversation_session("s-exact").await.unwrap();
    let cs_read = cs.read().await;
    assert_eq!(
        cs_read.messages().len(),
        5,
        "messages should remain unchanged at the limit"
    );
}

/// After truncation, `load_compact_inputs` reads from the same (truncated)
/// persistent history — verifying the single source of truth principle.
#[tokio::test]
async fn test_auto_compact_single_source_of_truth() {
    let sm = make_sm();
    insert_session_with_messages(&sm, "s-sot", 20).await;
    let config = CompactConfig {
        max_history_messages: Some(5),
        chars_per_token: 0.25,
        ..Default::default()
    };
    let handler = handler_no_output(&sm, config);
    handler.check_and_run_auto_compact("s-sot").await;

    // Read persistent history.
    let cs = sm.get_conversation_session("s-sot").await.unwrap();
    let persistent_len = cs.read().await.messages().len();

    // Load compact inputs — should reflect the same truncated history.
    let inputs = crate::session_manager::compact::load_compact_inputs(&sm, "s-sot").await;
    let Some((_model, llm_messages, _stats)) = inputs else {
        panic!("load_compact_inputs should return Some for existing session");
    };

    // llm_messages are filtered to user/assistant only, so its count
    // should be <= persistent_len but the persistent history is the
    // authoritative source.
    assert_eq!(
        persistent_len, 5,
        "persistent history should be exactly max_history_messages"
    );
    assert!(
        llm_messages.len() <= persistent_len,
        "llm_messages count should not exceed persistent history"
    );
}
