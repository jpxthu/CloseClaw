//! Execution context for slash command side effects.
//!
//! [`SideEffectContext`] encapsulates the session reference and reply channel
//! so that each [`SlashResult`](super::SlashResult) variant can complete its
//! own side effects without the Gateway enumerating every variant.

use std::sync::Arc;

use crate::gateway::session_manager::SessionManager;

/// Action produced by [`SlashResult::execute`](super::SlashResult::execute)
/// for the Gateway to dispatch.
#[derive(Debug)]
pub enum ReplyAction {
    /// Send a text reply to the user.
    Reply(String),
    /// Trigger manual compaction (Gateway delegates to session_handler).
    TriggerCompact { instruction: Option<String> },
    /// No action needed.
    Nothing,
}

/// Execution context for [`SlashResult::execute`](super::SlashResult::execute).
///
/// Encapsulates the session reference, channel identity, and reply channel so
/// that each `SlashResult` variant can complete its side effects internally.
pub struct SideEffectContext {
    /// The current session ID.
    pub session_id: String,
    /// The channel identifier (e.g. `"feishu"`).
    pub channel: String,
    /// Session manager for accessing conversation sessions.
    pub session_manager: Arc<SessionManager>,
    /// Channel to send reply actions back to the Gateway.
    reply_tx: tokio::sync::mpsc::Sender<ReplyAction>,
}

impl SideEffectContext {
    pub fn new(
        session_id: String,
        channel: String,
        session_manager: Arc<SessionManager>,
        reply_tx: tokio::sync::mpsc::Sender<ReplyAction>,
    ) -> Self {
        Self {
            session_id,
            channel,
            session_manager,
            reply_tx,
        }
    }

    /// Send a text reply to the user.
    pub async fn reply(&self, text: String) {
        let _ = self.reply_tx.send(ReplyAction::Reply(text)).await;
    }

    /// Trigger manual compaction.
    pub async fn trigger_compact(&self, instruction: Option<String>) {
        let _ = self
            .reply_tx
            .send(ReplyAction::TriggerCompact { instruction })
            .await;
    }

    /// Get the conversation session for the current session ID.
    pub async fn get_conversation_session(
        &self,
    ) -> Option<Arc<tokio::sync::RwLock<crate::llm::session::ConversationSession>>> {
        self.session_manager
            .get_conversation_session(&self.session_id)
            .await
    }
}
