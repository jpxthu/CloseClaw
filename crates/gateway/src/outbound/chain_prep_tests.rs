//! Unit tests for the shared outbound field-resolution helpers in
//! [`chain_prep`] — the per-field parsing behavior dimensions (issue #3804
//! Step 1.4): normal resolution, `MissingSessionId` error mapping,
//! `VerbosityLevel::default()` fallback, and thread-routing `None` fields.

use closeclaw_common::VerbosityLevel;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{PersistenceService, ReasoningLevel, SessionCheckpoint};
use std::sync::Arc;

use crate::persist_test_utils::SnapshotMockPersist;
use crate::{Gateway, GatewayConfig, SessionManager};

/// chat_id bound to every session created by the setups below
/// (`Session.agent_id` holds the chat_id per SessionManager convention).
const CHAT_ID: &str = "chat-1";

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-chain-prep".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

/// Gateway whose session map binds `session_id` → [`CHAT_ID`], optionally
/// backed by mock checkpoint storage.
async fn gw_with_session(
    session_id: &str,
    persist: Option<Arc<dyn PersistenceService>>,
) -> Gateway {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        persist,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: CHAT_ID.to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    Gateway::new_for_tests(config, sm)
}

/// Register a conversation session with the given verbosity level.
async fn insert_conversation_session(gw: &Gateway, session_id: &str, level: VerbosityLevel) {
    let cs = ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        std::env::temp_dir(),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    cs_arc.write().await.set_verbosity_level(level);
    gw.session_manager
        .conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), cs_arc);
}

/// Gateway whose checkpoint for `session_id` carries the given thread
/// routing fields (in-memory mock persistence).
async fn gw_with_thread_checkpoint(
    session_id: &str,
    thread_id: Option<&str>,
    reply_ref: Option<&str>,
) -> Gateway {
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.thread_id = thread_id.map(String::from);
    cp.reply_ref = reply_ref.map(String::from);
    let persist = Arc::new(SnapshotMockPersist::new());
    persist
        .checkpoints
        .lock()
        .await
        .insert(session_id.to_string(), cp);
    gw_with_session(session_id, Some(persist)).await
}

// ── resolve_outbound_chat_id ─────────────────────────────────────────────

/// Normal path: an existing session resolves to its chat_id.
#[tokio::test]
async fn test_resolve_outbound_chat_id_returns_session_chat_id() {
    let gw = gw_with_session("sess-cp-chat", None).await;
    let chat_id = gw.resolve_outbound_chat_id("sess-cp-chat").await.unwrap();
    assert_eq!(chat_id, CHAT_ID);
}

/// Error path: a session absent from the session map maps to
/// `GatewayError::MissingSessionId`.
#[tokio::test]
async fn test_resolve_outbound_chat_id_missing_session_maps_to_missing_session_id() {
    let gw = gw_with_session("sess-cp-chat", None).await;
    let err = gw
        .resolve_outbound_chat_id("sess-cp-ghost")
        .await
        .unwrap_err();
    assert!(
        matches!(err, crate::GatewayError::MissingSessionId),
        "expected MissingSessionId, got {err:?}"
    );
}

// ── resolve_outbound_verbosity_level ─────────────────────────────────────

/// Normal path: an existing conversation session's configured level is
/// returned as-is.
#[tokio::test]
async fn test_resolve_outbound_verbosity_level_reads_conversation_session() {
    let gw = gw_with_session("sess-cp-verb", None).await;
    // Off is deliberately distinct from the default (Full) so the assert
    // cannot pass via the fallback path.
    insert_conversation_session(&gw, "sess-cp-verb", VerbosityLevel::Off).await;
    let level = gw.resolve_outbound_verbosity_level("sess-cp-verb").await;
    assert_eq!(level, VerbosityLevel::Off);
}

/// Boundary: a session with no conversation session (or an unknown id) falls
/// back to the documented default verbosity level (`Full`).
#[tokio::test]
async fn test_resolve_outbound_verbosity_level_defaults_without_conversation_session() {
    let gw = gw_with_session("sess-cp-verb-none", None).await;
    let level = gw
        .resolve_outbound_verbosity_level("sess-cp-verb-none")
        .await;
    assert_eq!(level, VerbosityLevel::Full);
    // Unknown session id (absent from both tables) → same fallback.
    let ghost = gw.resolve_outbound_verbosity_level("sess-cp-ghost").await;
    assert_eq!(ghost, VerbosityLevel::Full);
}

// ── resolve_outbound_thread_routing ──────────────────────────────────────

/// Normal path: thread_id / reply_ref are read back from the session
/// checkpoint.
#[tokio::test]
async fn test_resolve_outbound_thread_routing_reads_checkpoint_fields() {
    let gw = gw_with_thread_checkpoint("sess-cp-route", Some("th-1"), Some("ref-9")).await;
    let routing = gw.resolve_outbound_thread_routing("sess-cp-route").await;
    assert_eq!(routing.thread_id.as_deref(), Some("th-1"));
    assert_eq!(routing.reply_ref.as_deref(), Some("ref-9"));
}

/// Boundary: a checkpoint without thread fields (or no checkpoint at all)
/// resolves both fields to `None`.
#[tokio::test]
async fn test_resolve_outbound_thread_routing_none_fields_without_thread_fields() {
    let gw = gw_with_thread_checkpoint("sess-cp-route-plain", None, None).await;
    let routing = gw
        .resolve_outbound_thread_routing("sess-cp-route-plain")
        .await;
    assert_eq!(routing.thread_id, None);
    assert_eq!(routing.reply_ref, None);
    // Session without any checkpoint entry → same None resolution.
    let ghost = gw.resolve_outbound_thread_routing("sess-cp-ghost").await;
    assert_eq!(ghost.thread_id, None);
    assert_eq!(ghost.reply_ref, None);
}

// ── combined normal path ─────────────────────────────────────────────────

/// Normal path: for one fully-populated session (session map + conversation
/// session + checkpoint), the three helpers resolve all four dispatch fields
/// coherently — the shape both outbound entries rely on.
#[tokio::test]
async fn test_resolve_outbound_fields_normal_path_all_four_fields() {
    let persist = {
        let mut cp = SessionCheckpoint::new("sess-cp-all".to_string());
        cp.thread_id = Some("th-all".to_string());
        cp.reply_ref = Some("ref-all".to_string());
        let persist = Arc::new(SnapshotMockPersist::new());
        persist
            .checkpoints
            .lock()
            .await
            .insert("sess-cp-all".into(), cp);
        persist as Arc<dyn PersistenceService>
    };
    let gw = gw_with_session("sess-cp-all", Some(persist)).await;
    insert_conversation_session(&gw, "sess-cp-all", VerbosityLevel::Normal).await;

    let chat_id = gw.resolve_outbound_chat_id("sess-cp-all").await.unwrap();
    let verbosity = gw.resolve_outbound_verbosity_level("sess-cp-all").await;
    let routing = gw.resolve_outbound_thread_routing("sess-cp-all").await;

    assert_eq!(chat_id, CHAT_ID);
    assert_eq!(verbosity, VerbosityLevel::Normal);
    assert_eq!(routing.thread_id.as_deref(), Some("th-all"));
    assert_eq!(routing.reply_ref.as_deref(), Some("ref-all"));
}
