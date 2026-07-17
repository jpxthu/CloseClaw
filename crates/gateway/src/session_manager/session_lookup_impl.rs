use async_trait::async_trait;

use super::SessionManager;
use closeclaw_common::{ModeTransition, PendingMessage, SessionLookup};

#[async_trait]
impl SessionLookup for SessionManager {
    async fn get_parent_of(&self, child_id: &str) -> Option<String> {
        SessionManager::get_parent_of(self, child_id).await
    }

    async fn get_chat_id(&self, session_id: &str) -> Option<String> {
        SessionManager::get_chat_id(self, session_id).await
    }

    async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String> {
        SessionManager::push_pending_message(self, session_id, msg).await
    }

    async fn get_plan_state(&self, session_id: &str) -> Option<closeclaw_common::PlanState> {
        SessionManager::get_plan_state(self, session_id).await
    }

    async fn set_plan_state(&self, session_id: &str, plan_state: closeclaw_common::PlanState) {
        SessionManager::set_plan_state(self, session_id, plan_state).await;
    }

    async fn set_session_mode(&self, session_id: &str, mode: closeclaw_common::SessionMode) {
        if let Some(cs) = self.get_conversation_session(session_id).await {
            cs.write().await.set_session_mode(mode);
        }
    }

    async fn set_pending_mode_transition(&self, session_id: &str, transition: ModeTransition) {
        if let Some(cs) = self.get_conversation_session(session_id).await {
            cs.write().await.set_pending_mode_transition(transition);
        }
    }
}
