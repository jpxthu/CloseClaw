//! Channel-level session routing helpers.
//!
//! These methods extend `SessionManager` with channel→session mapping
//! so that `/new` can force a fresh session per channel while the old
//! session is preserved for recovery.

use super::SessionManager;
use crate::gateway::Session;
use crate::llm::session::ConversationSession;
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
    /// Returns the new session_id.
    pub async fn force_new_for_channel(&self, channel: &str, agent_id: &str) -> String {
        // Generate a unique session id using timestamp + channel
        let session_id = format!(
            "{}-{}-{}",
            channel,
            agent_id,
            chrono::Utc::now().timestamp_millis()
        );

        // Compute workdir
        let workdir_path = if let Some(ref workspace_dir) = self.workspace_dir {
            crate::session::workspace::ensure_workspace_dir(workspace_dir, agent_id, agent_id)
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        } else {
            std::path::PathBuf::from("/tmp")
        };

        // Create ConversationSession
        let conv_session =
            ConversationSession::new(session_id.clone(), "default".to_string(), workdir_path)
                .with_reasoning_level(self.default_reasoning_level);
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

        session_id
    }
}
