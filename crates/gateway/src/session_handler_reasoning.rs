//! Gateway pre-call reasoning level resolution.
//!
//! Provides an async helper that resolves the effective reasoning level
//! for a session *before* each LLM invocation, combining the session's
//! requested level with provider model knowledge and heuristic fallbacks.
//!
//! See `docs/design/session/llm-session-enhancements.md` §实际生效档位.

use super::session_handler_announce::resolve_effective_reasoning_level;
use super::session_manager::SessionManager;

/// Resolve effective reasoning level and write it back to the session.
///
/// Fetches the session, reads model + requested level, resolves via
/// provider knowledge / heuristic fallback, logs any downgrade, and
/// writes the effective level back. Callers must drop any locks on the
/// conversation session before calling this.
pub(crate) async fn resolve_before_llm_call(session_manager: &SessionManager, session_id: &str) {
    let Some(cs) = session_manager.get_conversation_session(session_id).await else {
        return;
    };
    let (model, requested) = {
        let r = cs.read().await;
        (r.model().to_string(), r.reasoning_level())
    };
    let gw = session_manager.get_gateway_ref().await;
    let Some(kb) = gw.as_ref().and_then(|g| g.model_knowledge()) else {
        return;
    };
    let effective = resolve_effective_reasoning_level(&model, requested, kb);
    if effective != requested {
        tracing::info!(
            session_id, from = ?requested, to = ?effective,
            model = %model, "reasoning level downgraded (pre-call resolve)"
        );
    }
    cs.write().await.set_effective_reasoning_level(effective);
}
