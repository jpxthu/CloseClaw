//! Shared helper for rebuilding system prompts after compaction.
//!
//! Both `session_handler.rs` and `session_handler_compaction.rs` need to
//! rebuild the system prompt after compaction. This module provides the
//! shared implementation to avoid code duplication.

use crate::session_manager::SessionManager;

/// Rebuild the system prompt for a session using the session manager's
/// builder and overrides. Delegates to `ConversationSession::rebuild_system_prompt`.
pub async fn rebuild_system_prompt_for_session(sm: &SessionManager, session_id: &str) {
    let cs = match sm.get_conversation_session(session_id).await {
        Some(cs) => cs,
        None => return,
    };
    let agent_id = {
        let sessions = sm.sessions.read().await;
        match sessions.get(session_id) {
            Some(session) => session.agent_id.clone(),
            None => return,
        }
    };
    let builder = match sm.get_system_prompt_builder().await {
        Some(b) => b,
        None => {
            tracing::debug!(
                session_id,
                "no system prompt builder configured, skipping rebuild"
            );
            return;
        }
    };
    let overrides = sm.get_prompt_overrides().await;
    let mut cs = cs.write().await;
    cs.rebuild_system_prompt(session_id, &agent_id, builder.as_ref(), overrides.as_ref())
        .await;
}
