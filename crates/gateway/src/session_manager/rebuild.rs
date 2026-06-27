//! System prompt rebuild logic for `SessionManager`.
//!
//! Extracted from `session_manager.rs` to keep the file under the
//! 500-line hard limit.

use super::SessionManager;

impl SessionManager {
    /// Rebuild the system prompt for an existing session.
    /// Called after compaction to pick up skill/config changes.
    /// Lock Safety: acquires its own write lock; callers must NOT hold
    /// any external write guard on the same session.
    pub async fn rebuild_system_prompt(&self, session_id: &str) {
        let cs = match self.get_conversation_session(session_id).await {
            Some(cs) => cs,
            None => return,
        };
        let agent_id = {
            let sessions = self.sessions.read().await;
            match sessions.get(session_id) {
                Some(session) => session.agent_id.clone(),
                None => return,
            }
        };

        let builder = match self.system_prompt_builder.read().await.clone() {
            Some(b) => b,
            None => {
                tracing::debug!(
                    session_id,
                    "no system prompt builder configured, skipping rebuild"
                );
                return;
            }
        };

        let overrides = self.prompt_overrides.read().await.clone();
        let prompt = builder
            .build_prompt(session_id, &agent_id, overrides.as_ref())
            .await;

        let mut cs = cs.write().await;
        cs.replace_system_prompt(prompt);
    }
}
