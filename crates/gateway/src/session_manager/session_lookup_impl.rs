use async_trait::async_trait;
use std::sync::Arc;

use super::SessionManager;
use closeclaw_common::{PendingMessage, SessionLookup, SlashSessionQuery};
use closeclaw_session::llm_session::ChatSession;

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
            cs.write().await.set_session_mode(
                mode,
                closeclaw_session::llm_session::mode_transition::ModeChangeSource::Automatic,
            );
        }
    }

    async fn set_pending_session_mode(
        &self,
        session_id: &str,
        mode: closeclaw_common::SessionMode,
    ) {
        if let Some(cs) = self.get_conversation_session(session_id).await {
            cs.read().await.set_pending_session_mode(mode);
        }
    }

    async fn clear_plan_state(&self, session_id: &str) {
        SessionManager::clear_plan_state(self, session_id).await;
    }
}

/// Helper: get conversation session by ID.
type ConvSession = Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>;

async fn get_cs(mgr: &SessionManager, session_id: &str) -> Option<ConvSession> {
    mgr.get_conversation_session(session_id).await
}

#[async_trait]
impl SlashSessionQuery for SessionManager {
    async fn get_plan_state(&self, session_id: &str) -> Option<closeclaw_common::PlanState> {
        SessionManager::get_plan_state(self, session_id).await
    }

    async fn set_plan_state(&self, session_id: &str, plan_state: closeclaw_common::PlanState) {
        SessionManager::set_plan_state(self, session_id, plan_state).await;
    }

    async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String> {
        SessionManager::push_pending_message(self, session_id, msg).await
    }

    async fn trigger_manual_background(&self, session_id: &str) -> Result<bool, String> {
        SessionManager::trigger_manual_background(self, session_id).await
    }

    async fn set_workflow_run(
        &self,
        session_id: &str,
        run: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<(), String> {
        let typed_run = run.map(|r| {
            r.downcast::<closeclaw_workflow::run::WorkflowRun>()
                .expect("set_workflow_run: downcast to WorkflowRun failed")
                .as_ref()
                .clone()
        });
        SessionManager::set_workflow_run(self, session_id, typed_run).await
    }

    async fn invalidate_static_cache(&self) {
        SessionManager::invalidate_static_cache(self).await;
    }

    async fn rebuild_system_prompt_for_session(&self, session_id: &str) {
        SessionManager::rebuild_system_prompt_for_session(self, session_id).await;
    }

    async fn add_system_append(&self, session_id: &str, content: String) {
        let Some(cs) = get_cs(self, session_id).await else {
            return;
        };
        let mut guard = cs.write().await;
        guard.add_system_append(content);
    }

    async fn get_model(&self, session_id: &str) -> Option<String> {
        let cs = get_cs(self, session_id).await?;
        let guard = cs.read().await;
        Some(guard.model().to_owned())
    }

    async fn get_reasoning_level(&self, session_id: &str) -> Option<String> {
        let cs = get_cs(self, session_id).await?;
        let guard = cs.read().await;
        Some(guard.effective_reasoning_level().to_string())
    }

    async fn get_verbosity_level(&self, session_id: &str) -> Option<String> {
        let cs = get_cs(self, session_id).await?;
        let guard = cs.read().await;
        Some(guard.verbosity_level().to_string())
    }

    async fn get_session_mode(&self, session_id: &str) -> Option<closeclaw_common::SessionMode> {
        let cs = get_cs(self, session_id).await?;
        let guard = cs.read().await;
        Some(guard.session_mode())
    }

    async fn get_workdir(&self, session_id: &str) -> Option<std::path::PathBuf> {
        let cs = get_cs(self, session_id).await?;
        let guard = cs.read().await;
        Some(guard.workdir().to_path_buf())
    }

    async fn get_system_appends(&self, session_id: &str) -> Vec<String> {
        let Some(cs) = get_cs(self, session_id).await else {
            return Vec::new();
        };
        let guard = cs.read().await;
        guard.system_appends().to_vec()
    }

    async fn set_workdir(&self, session_id: &str, path: std::path::PathBuf) {
        let Some(cs) = get_cs(self, session_id).await else {
            return;
        };
        let mut guard = cs.write().await;
        guard.set_workdir(path);
    }

    async fn is_llm_busy(&self, session_id: &str) -> bool {
        let Some(cs) = get_cs(self, session_id).await else {
            return false;
        };
        let guard = cs.read().await;
        guard.is_llm_busy()
    }

    async fn get_stats(&self, session_id: &str) -> Option<(usize, usize, usize, usize)> {
        let cs = get_cs(self, session_id).await?;
        let guard = cs.read().await;
        let stats = guard.stats();
        Some((
            stats.total_tokens as usize,
            stats.total_prompt_tokens as usize,
            stats.total_cache_read_tokens as usize,
            stats.total_cache_write_tokens as usize,
        ))
    }

    async fn get_last_cache_break(&self, session_id: &str) -> Option<String> {
        let cs = get_cs(self, session_id).await?;
        let guard = cs.read().await;
        guard.last_cache_break().map(|cb| cb.format_notification())
    }

    async fn get_active_child_count(&self, session_id: &str) -> usize {
        let Some(cs) = get_cs(self, session_id).await else {
            return 0;
        };
        let guard = cs.read().await;
        let child_handles = guard
            .child_handles
            .read()
            .unwrap_or_else(|e| e.into_inner());
        child_handles
            .values()
            .filter(|w| w.upgrade().is_some())
            .count()
    }
}
