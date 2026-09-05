//! Gateway pre-call reasoning level resolution.
//!
//! Provides an async helper that resolves the effective reasoning level
//! for a session *before* each LLM invocation, combining the session's
//! requested level with provider model knowledge and heuristic fallbacks.
//!
//! See `docs/design/session/llm-session-enhancements.md` §实际生效档位.

use closeclaw_llm::ProviderModelKnowledge;

use super::session_manager::SessionManager;

/// Resolve effective reasoning level and write it back to the session.
///
/// Called before each LLM invocation. Looks up the model in `knowledge`,
/// applies provider-specific downgrade logic, and writes the result back
/// via `set_effective_reasoning_level`.
///
/// - When `knowledge` is `None`, the session is left untouched (fallback
///   to the `effective_reasoning_level()` getter's default behavior).
/// - When downgrade occurs (effective ≠ requested), an `info!` log is
///   emitted with from/to/provider dimensions.
#[allow(dead_code)] // Will be wired in Step 1.2
pub(crate) async fn resolve_before_llm(
    session_manager: &SessionManager,
    session_id: &str,
    knowledge: Option<&ProviderModelKnowledge>,
) {
    let Some(kb) = knowledge else {
        return;
    };
    let Some(cs) = session_manager.get_conversation_session(session_id).await else {
        return;
    };
    let mut cs_write = cs.write().await;
    let requested = cs_write.reasoning_level();
    let model = cs_write.model().to_string();
    let effective =
        super::session_handler_announce::resolve_effective_reasoning_level(&model, requested, kb);
    if effective != requested {
        tracing::info!(
            session_id,
            from = ?requested,
            to = ?effective,
            model = %model,
            "reasoning level downgraded (pre-call resolve)"
        );
    }
    cs_write.set_effective_reasoning_level(effective);
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
        // glm-5.1: Toggle { on: true }, requested Max → effective High
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("glm-5.1", ReasoningLevel::Max).await;

        resolve_before_llm(&sm, "test-session", Some(&kb)).await;

        let cs = sm.get_conversation_session("test-session").await.unwrap();
        let cs_read = cs.read().await;
        assert_eq!(cs_read.effective_reasoning_level(), ReasoningLevel::High);
    }

    #[tokio::test]
    async fn test_kb_miss_non_anthropic_keeps_requested() {
        // Unknown model, not anthropic → no downgrade, stays Max
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("unknown-model", ReasoningLevel::Max).await;

        resolve_before_llm(&sm, "test-session", Some(&kb)).await;

        let cs = sm.get_conversation_session("test-session").await.unwrap();
        let cs_read = cs.read().await;
        assert_eq!(cs_read.effective_reasoning_level(), ReasoningLevel::Max);
    }

    #[tokio::test]
    async fn test_kb_miss_anthropic_heuristic_downgrades() {
        // claude model, not in kb → heuristic: non-High → High
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("claude-3-opus", ReasoningLevel::Medium).await;

        resolve_before_llm(&sm, "test-session", Some(&kb)).await;

        let cs = sm.get_conversation_session("test-session").await.unwrap();
        let cs_read = cs.read().await;
        assert_eq!(cs_read.effective_reasoning_level(), ReasoningLevel::High);
    }

    #[tokio::test]
    async fn test_knowledge_none_does_not_write_back() {
        // knowledge=None → session stays untouched, effective stays None (fallback)
        let sm = make_sm_with_session("glm-5.1", ReasoningLevel::High).await;

        resolve_before_llm(&sm, "test-session", None).await;

        let cs = sm.get_conversation_session("test-session").await.unwrap();
        let cs_read = cs.read().await;
        // effective_reasoning_level() returns fallback to requested when None
        assert_eq!(cs_read.effective_reasoning_level(), ReasoningLevel::High);
    }

    #[tokio::test]
    async fn test_kb_hit_no_downgrade_sets_effective() {
        // glm-5.1: Toggle { on: true }, requested Low → effective Low (no downgrade)
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("glm-5.1", ReasoningLevel::Low).await;

        resolve_before_llm(&sm, "test-session", Some(&kb)).await;

        let cs = sm.get_conversation_session("test-session").await.unwrap();
        let cs_read = cs.read().await;
        assert_eq!(cs_read.effective_reasoning_level(), ReasoningLevel::Low);
    }

    #[tokio::test]
    async fn test_session_not_found_no_panic() {
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("glm-5.1", ReasoningLevel::High).await;
        // session_id not in manager — should not panic
        resolve_before_llm(&sm, "nonexistent", Some(&kb)).await;
    }

    #[tokio::test]
    async fn test_deepseek_levels_kb_hit() {
        // deepseek-v4-flash: Levels { off: true, base: true, reasoner: false }
        // Max → High (reasoner=false)
        let kb = ProviderModelKnowledge::new();
        let sm = make_sm_with_session("deepseek-v4-flash", ReasoningLevel::Max).await;

        resolve_before_llm(&sm, "test-session", Some(&kb)).await;

        let cs = sm.get_conversation_session("test-session").await.unwrap();
        let cs_read = cs.read().await;
        assert_eq!(cs_read.effective_reasoning_level(), ReasoningLevel::High);
    }
}
