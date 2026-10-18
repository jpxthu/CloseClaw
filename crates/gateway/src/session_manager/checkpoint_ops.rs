//! Checkpoint / persistence operations for [`SessionManager`].
//!
//! Extracted from `session_manager.rs` to keep file size under the
//! 1000-line limit while preserving full doc comments.

use super::session_helpers;
use super::SessionManager;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_session::persistence::{
    PendingMessage, PersistenceError, SessionCheckpoint, SessionStatus,
};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

impl SessionManager {
    /// Attempt to restore an archived session.
    /// Returns true iff restoration was attempted and succeeded.
    pub(super) async fn try_restore_archived_session(
        &self,
        session_id: &str,
        channel: &str,
    ) -> bool {
        let storage_arc = {
            let cm_guard = self.checkpoint_manager.read().await;
            match cm_guard.as_ref() {
                Some(cm) => Arc::clone(cm.storage_arc()),
                None => return false,
            }
        };
        let result =
            session_helpers::try_restore_archived_session_inner(&storage_arc, session_id, channel)
                .await;
        // Store the notification chat_id for Gateway to send via outbound chain.
        if let Some(chat_id) = result.notification_chat_id {
            let mut pending = self.pending_restore_notifications.write().await;
            pending.insert(session_id.to_string(), (chat_id, None));
        }
        result.restored
    }

    /// Take the pending restore notification for a session.
    /// Returns `(chat_id, Option<custom_message>)` if a restore notification
    /// is pending. When `custom_message` is `None`, the caller uses the
    /// default "正在恢复会话..." text; otherwise the custom message is used.
    pub async fn take_restore_notification(
        &self,
        session_id: &str,
    ) -> Option<(String, Option<String>)> {
        let mut pending = self.pending_restore_notifications.write().await;
        pending.remove(session_id)
    }

    /// Queue a workflow blocked notification for a session.
    /// The notification is consumed by
    /// [`take_workflow_notification`](Self::take_workflow_notification) after
    /// the Gateway sends it through the outbound chain.
    pub async fn queue_workflow_notification(&self, session_id: &str, message: String) {
        let mut pending = self.pending_workflow_notifications.write().await;
        pending.insert(session_id.to_string(), message);
    }

    /// Take the pending workflow blocked notification for a session.
    /// Returns the notification message text if one is pending.
    pub async fn take_workflow_notification(&self, session_id: &str) -> Option<String> {
        let mut pending = self.pending_workflow_notifications.write().await;
        pending.remove(session_id)
    }

    /// Update thread_id and reply_ref in a session's checkpoint.
    pub(super) async fn update_checkpoint_fields(
        &self,
        session_id: &str,
        thread_id: &Option<String>,
        reply_ref: &Option<String>,
    ) {
        if let Some(cm) = self.checkpoint_manager.read().await.as_ref() {
            session_helpers::update_checkpoint_fields(
                cm.as_ref(),
                session_id,
                thread_id,
                reply_ref,
            )
            .await;
        }
    }

    /// Update `last_user_activity_at` and `last_message_at` for a user message.
    ///
    /// Called from the message handler when a user message arrives.
    /// Updates both timestamps to now and persists the checkpoint.
    pub async fn update_checkpoint_user_activity(&self, session_id: &str) {
        let cm_guard = self.checkpoint_manager.read().await;
        let Some(cm) = cm_guard.as_ref() else {
            warn!(
                session_id = %session_id,
                "storage not available, skipping user activity update"
            );
            return;
        };
        session_helpers::update_checkpoint_user_activity(cm.as_ref(), session_id).await;
    }

    /// Force a WAL checkpoint via the persistence backend.
    ///
    /// Should be called after `flush_all` in Phase 4 to ensure all
    /// session writes are durable on disk.
    pub async fn sync_storage(&self) -> Result<(), PersistenceError> {
        let cm = self.checkpoint_manager.read().await;
        let Some(cm) = cm.as_ref() else {
            return Ok(());
        };
        cm.storage().sync().await
    }

    /// Explicitly close the storage backend and release resources.
    ///
    /// Called during Phase 6 of daemon shutdown. Releases persistent
    /// connections or file handles held by the storage backend.
    pub async fn close_storage(&self) -> Result<(), PersistenceError> {
        let cm = self.checkpoint_manager.read().await;
        let Some(cm) = cm.as_ref() else {
            return Ok(());
        };
        cm.storage().close().await
    }

    /// Flush all active sessions to persistence.
    /// Returns the number of sessions successfully saved.
    pub async fn flush_all(&self, _mode: ShutdownMode) -> Result<usize, PersistenceError> {
        let cm = self.checkpoint_manager.read().await;
        let Some(cm) = cm.as_ref() else {
            return Ok(0);
        };
        let sessions = self.sessions.read().await;
        // Collect session ids first to avoid holding sessions lock across I/O
        let session_ids: Vec<String> = sessions.keys().cloned().collect();
        drop(sessions);

        // Collect pending messages and user_appends using async RwLock read.
        let mut pending_map: HashMap<String, Vec<PendingMessage>> = HashMap::new();
        let mut appends_map: HashMap<String, Vec<String>> = HashMap::new();
        {
            let conv_sessions = self.conversation_sessions.read().await;
            for sid in &session_ids {
                if let Some(cs) = conv_sessions.get(sid) {
                    let cs = cs.read().await;
                    pending_map.insert(sid.clone(), cs.get_pending_messages());
                    appends_map.insert(sid.clone(), cs.user_system_appends().to_vec());
                }
            }
        } // Drop conv_sessions read lock before checkpoint persistence
        let sessions = self.sessions.read().await;
        let mut saved = 0;
        for (session_id, session) in sessions.iter() {
            let pending = pending_map.get(session_id).cloned().unwrap_or_default();
            // Load existing checkpoint to preserve fields like thread_id (Bug #904).
            let mut cp = match cm.load(session_id).await {
                Ok(Some(mut cp)) => {
                    // Update fields from active session state
                    cp.status = SessionStatus::Active;
                    cp.platform = Some(session.channel.clone());
                    cp.peer_id = Some(session.agent_id.clone());
                    cp.agent_id = Some(session.agent_id.clone());
                    cp.outbound_pending = pending;
                    cp
                }
                // No existing checkpoint — create a fresh one
                _ => SessionCheckpoint::new(session_id.clone())
                    .with_status(SessionStatus::Active)
                    .with_platform(session.channel.clone())
                    .with_peer_id(session.agent_id.clone())
                    .with_agent_id(session.agent_id.clone())
                    .with_outbound_pending(pending),
            };
            // Sync per-session append-section list from ConversationSession
            // (issue #860: archived session restore preserves append content).
            if let Some(appends) = appends_map.get(session_id) {
                cp.user_appends = appends.clone();
            }
            if cm.save_raw(&cp).await.is_ok() {
                saved += 1;
            } else {
                warn!(session_id = %session_id, "failed to save session checkpoint");
            }
        }
        drop(sessions);

        // Phase 4 cleanup: remove sessions from tracking tables after
        // all checkpoint persistence is complete. This ensures the
        // "fallback persistence" path actually finds sessions that
        // were stopped in Phase 2 but not yet removed from tracking.
        for session_id in &session_ids {
            self.remove_session(session_id).await;
        }

        Ok(saved)
    }

    /// Get the thread_id for a session from its checkpoint.
    /// Returns None if the session has no thread_id or the storage is not available.
    pub async fn get_thread_id(&self, session_id: &str) -> Option<String> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.thread_id,
            _ => None,
        }
    }

    pub async fn get_reply_ref(&self, session_id: &str) -> Option<String> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.reply_ref,
            _ => None,
        }
    }

    /// Get the sender_id (user ID) for a session from its checkpoint.
    pub async fn get_sender_id(&self, session_id: &str) -> Option<String> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.sender_id,
            _ => None,
        }
    }

    /// Get the plan_state from the session checkpoint.
    /// Returns None if the session has no plan_state or storage is unavailable.
    pub async fn get_plan_state(&self, session_id: &str) -> Option<closeclaw_common::PlanState> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.plan_state,
            _ => None,
        }
    }
    /// Set plan_state on the session checkpoint.
    /// Writes the checkpoint first; updates ConversationSession on success.
    pub async fn set_plan_state(&self, session_id: &str, plan_state: closeclaw_common::PlanState) {
        // Write checkpoint first; skip ConversationSession if checkpoint fails.
        let plan_file_path = if plan_state.plan_file_path.is_empty() {
            None
        } else {
            Some(plan_state.plan_file_path.clone())
        };
        let cm = {
            let guard = self.checkpoint_manager.read().await;
            match guard.as_ref() {
                Some(cm) => Arc::clone(cm),
                None => return,
            }
        };
        let mut cp = match cm.load(session_id).await {
            Ok(Some(cp)) => cp,
            _ => return,
        };
        cp.plan_state = Some(plan_state);
        if let Err(e) = cm.save_raw(&cp).await {
            tracing::warn!(session_id = %session_id, "failed to save plan_state: {}", e);
            return;
        }
        // Sync plan_file_path to ConversationSession for Auto Mode injection.
        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs = cs.write().await;
            cs.set_plan_file_path(plan_file_path);
        }
    }
    /// Clear plan state when exiting Plan Mode (e.g. `/mode normal`, `/auto`).
    pub async fn clear_plan_state(&self, session_id: &str) {
        // Clone Arc before dropping the guard to avoid holding it across await.
        let cm = {
            let guard = self.checkpoint_manager.read().await;
            guard.as_ref().map(Arc::clone)
        };
        if let Some(cm) = cm {
            if session_helpers::clear_plan_state_checkpoint(&cm, session_id)
                .await
                .is_err()
            {
                warn!(session_id = %session_id, "failed to clear plan_state checkpoint");
                return;
            }
        }
        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs = cs.write().await;
            cs.set_plan_file_path(None);
        }
    }
}
