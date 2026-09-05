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

/// Test-only variant: resolve with explicit knowledge (no gateway needed).
#[cfg(test)]
use closeclaw_llm::ProviderModelKnowledge;

#[cfg(test)]
pub(crate) async fn resolve_before_llm_call_with_kb(
    session_manager: &SessionManager,
    session_id: &str,
    knowledge: &ProviderModelKnowledge,
) {
    let Some(cs) = session_manager.get_conversation_session(session_id).await else {
        return;
    };
    let (model, requested) = {
        let r = cs.read().await;
        (r.model().to_string(), r.reasoning_level())
    };
    let effective = resolve_effective_reasoning_level(&model, requested, knowledge);
    if effective != requested {
        tracing::info!(
            session_id, from = ?requested, to = ?effective,
            model = %model, "reasoning level downgraded (pre-call resolve)"
        );
    }
    cs.write().await.set_effective_reasoning_level(effective);
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_session::llm_session::ConversationSession;
    use closeclaw_session::persistence::ReasoningLevel;
    use std::sync::Arc;

    fn test_config() -> crate::GatewayConfig {
        crate::GatewayConfig {
            name: "test".to_string(),
            ..Default::default()
        }
    }

    fn make_session_manager() -> SessionManager {
        SessionManager::new(&test_config(), None, None, ReasoningLevel::default())
    }

    async fn make_sm_with_session(model: &str, level: ReasoningLevel) -> SessionManager {
        let sm = make_session_manager();
        let cs = ConversationSession::new(
            "test-session".to_string(),
            model.to_string(),
            std::path::PathBuf::from("/tmp"),
        );
        sm.conversation_sessions.write().await.insert(
            "test-session".to_string(),
            Arc::new(tokio::sync::RwLock::new(cs)),
        );
        // Set the requested reasoning level if not default.
        if level != ReasoningLevel::default() {
            let cs = sm.get_conversation_session("test-session").await.unwrap();
            cs.write().await.set_reasoning_level(level);
        }
        sm
    }

    // ── write-back correctness ────────────────────────────────────────

    #[tokio::test]
    async fn test_kb_hit_downgrades() {
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("glm-5.1", ReasoningLevel::Max).await;
        resolve_before_llm_call_with_kb(&sm, "test-session", &kb).await;
        let cs = sm.get_conversation_session("test-session").await.unwrap();
        assert_eq!(
            cs.read().await.effective_reasoning_level(),
            ReasoningLevel::High
        );
    }

    #[tokio::test]
    async fn test_kb_miss_non_anthropic_keeps_requested() {
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("unknown-model", ReasoningLevel::Max).await;
        resolve_before_llm_call_with_kb(&sm, "test-session", &kb).await;
        let cs = sm.get_conversation_session("test-session").await.unwrap();
        assert_eq!(
            cs.read().await.effective_reasoning_level(),
            ReasoningLevel::Max
        );
    }

    #[tokio::test]
    async fn test_kb_miss_anthropic_heuristic_downgrades() {
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("claude-3-opus", ReasoningLevel::Medium).await;
        resolve_before_llm_call_with_kb(&sm, "test-session", &kb).await;
        let cs = sm.get_conversation_session("test-session").await.unwrap();
        assert_eq!(
            cs.read().await.effective_reasoning_level(),
            ReasoningLevel::High
        );
    }

    #[tokio::test]
    async fn test_knowledge_none_does_not_write_back() {
        let sm = make_sm_with_session("glm-5.1", ReasoningLevel::High).await;
        // Production helper: no gateway ref → knowledge is None → no write-back.
        resolve_before_llm_call(&sm, "test-session").await;
        let cs = sm.get_conversation_session("test-session").await.unwrap();
        assert_eq!(
            cs.read().await.effective_reasoning_level(),
            ReasoningLevel::High
        );
    }

    #[tokio::test]
    async fn test_session_not_found_no_panic() {
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("glm-5.1", ReasoningLevel::High).await;
        resolve_before_llm_call_with_kb(&sm, "nonexistent", &kb).await;
    }
}
