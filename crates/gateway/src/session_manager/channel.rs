//! Channel-level session routing helpers.
//!
//! These methods extend `SessionManager` with channel→session mapping
//! so that `/new` can force a fresh session per channel while the old
//! session is preserved for recovery.

use super::SessionManager;
use crate::Session;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{SessionCheckpoint, SessionStatus};
use std::sync::Arc;
use tokio::sync::RwLock;

impl SessionManager {
    /// Returns the active session_id for a channel, if any.
    #[allow(dead_code)]
    pub async fn active_session_for_channel(&self, channel: &str) -> Option<String> {
        let channel_map = self.channel_active_sessions.read().await;
        channel_map.get(channel).cloned()
    }

    /// Force-create a new session for the given channel, replacing the
    /// channel→session mapping. The old session is preserved in the
    /// sessions map for recovery but is no longer routed to by default.
    ///
    /// Cascade-terminates all active children of the old session
    /// before creating the new one (design doc §生命周期联动).
    ///
    /// Returns the new session_id.
    pub async fn force_new_for_channel(&self, channel: &str, agent_id: &str) -> String {
        // Cascade-kill children of the old session (if any) so they
        // don't outlive the parent. Per design doc §生命周期联动:
        // "父 session 正常结束: 所有仍活跃的子 session 被自动级联终止".
        if let Some(old_id) = self.active_session_for_channel(channel).await {
            self.cascade_kill_all_children(&old_id).await;
        }

        // Generate a unique session id using the standard format
        let session_id = super::session_helpers::generate_session_id(agent_id);
        // Compute session_key for key_registry
        let session_key = format!("{}:{}:{}", channel, agent_id, agent_id);

        // Compute workdir
        let workdir_path = if let Some(ref workspace_dir) = self.workspace_dir {
            closeclaw_session::workspace::ensure_workspace_dir(workspace_dir, agent_id, agent_id)
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        } else {
            std::path::PathBuf::from("/tmp")
        };

        // Create ConversationSession
        let mut conv_session =
            ConversationSession::new(session_id.clone(), "default".to_string(), workdir_path)
                .with_reasoning_level(self.default_reasoning_level);
        // Wire shutdown handle for busy-count tracking.
        if let Some(sh) = self.get_shutdown_handle().await {
            conv_session.set_shutdown_handle(sh);
        }
        // Inject LLM caller and system prompt builder for delegation.
        if let Some(caller) = self.get_llm_caller().await {
            conv_session.set_llm_caller(caller.clone());
            conv_session.init_health_checker(caller);
        }
        if let Some(builder) = self.get_system_prompt_builder().await {
            conv_session.set_system_prompt_builder(builder);
        }
        conv_session.set_prompt_overrides(self.get_prompt_overrides().await);
        // Inject snapshot meta store for persistence.
        self.inject_snapshot_meta_store(&session_id, &mut conv_session)
            .await;
        {
            let mut conv_sessions = self.conversation_sessions.write().await;
            conv_sessions.insert(session_id.clone(), Arc::new(RwLock::new(conv_session)));
        }

        // Create Session entry
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                session_id.clone(),
                Session {
                    id: session_id.clone(),
                    agent_id: agent_id.to_string(),
                    channel: channel.to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                    depth: 0,
                },
            );
        }

        // Update channel → session mapping
        {
            let mut channel_map = self.channel_active_sessions.write().await;
            channel_map.insert(channel.to_string(), session_id.clone());
        }

        // Update key_registry so resolve() routes to the new session
        {
            let mut registry = self.key_registry.write().await;
            registry.insert(session_key, session_id.clone());
        }

        // Persist checkpoint so rebuild_key_registry can reconstruct the
        // session_key after restart.  The /new command has no real
        // message.from, so we use agent_id as a placeholder.
        let mut cp = SessionCheckpoint::new(session_id.clone())
            .with_status(SessionStatus::Active)
            .with_platform(channel.to_string())
            .with_peer_id(agent_id.to_string())
            .with_agent_id(agent_id.to_string());
        cp.sender_id = Some(agent_id.to_string());
        if let Some(cm) = self.checkpoint_manager.read().await.as_ref() {
            if let Err(e) = cm.save_raw(&cp).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to save checkpoint for force_new_for_channel"
                );
            }
        }

        session_id
    }
}
