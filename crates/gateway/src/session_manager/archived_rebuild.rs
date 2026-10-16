//! In-memory archived-session prompt rebuild.
//!
//! Extracted from resolve.rs to keep file size under the 1000-line limit.

use super::SessionManager;
use crate::Message;
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::persistence::SessionCheckpoint;

impl SessionManager {
    /// Rebuild system prompt for archived session with in-memory ConversationSession.
    pub(super) async fn rebuild_archived_session_prompt(
        &self,
        session_id: &str,
        cp: &SessionCheckpoint,
        message: &Message,
    ) {
        let agent_id_for_rebuild = cp.agent_id.clone().unwrap_or_else(|| message.to.clone());
        let bootstrap_mode = self
            .query_agent_bootstrap_mode(&agent_id_for_rebuild)
            .await
            .unwrap_or(BootstrapMode::Full);
        let cs_arc = {
            let cs = self.conversation_sessions.read().await;
            cs.get(session_id).cloned()
        };
        if let Some(cs_arc) = cs_arc {
            let mut cs = cs_arc.write().await;
            // Inject system prompt builder (was already present).
            if let Some(builder) = self.get_system_prompt_builder().await {
                cs.set_system_prompt_builder(builder);
            }
            // Inject prompt overrides (missing — added for parity with new session path).
            cs.set_prompt_overrides(self.get_prompt_overrides().await);
            // Inject tool registry for injection chain
            // (design doc §注入链路的参数契约).
            if let Some(registry) = self.get_tool_registry().await {
                cs.set_tool_registry(registry);
            }
            // Inject dynamic prompt builder for per-request dynamic-layer injection.
            if let Some(dpb) = self.get_dynamic_prompt_builder().await {
                cs.set_dynamic_prompt_builder(dpb);
            }
            // Inject skill listing provider and agent-level skills whitelist.
            self.wire_skill_listing_deps(&mut cs, &agent_id_for_rebuild)
                .await;
            // Cache bootstrap mode on the session (was only queried, not cached).
            *cs = cs.clone().with_bootstrap_mode(bootstrap_mode);
            // Rebuild the system prompt (existing behavior).
            cs.rebuild_system_prompt(session_id, &agent_id_for_rebuild, Some(bootstrap_mode))
                .await;
            // Inject snapshot meta store for persistence.
            self.inject_snapshot_meta_store(session_id, &mut cs).await;
            // Inject checkpoint storage for pending-operation persistence.
            self.inject_checkpoint_storage(&mut cs).await;
            // Apply session config (git_status switch).
            if let Some(cfg) = self
                .get_session_config_for_agent(&agent_id_for_rebuild)
                .await
            {
                cs.set_git_status(cfg.is_git_status_enabled);
            }
        }
    }
}
