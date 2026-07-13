//! Compaction snapshot management for [`SessionManager`].
//!
//! Handles checkpoint persistence after compaction and pre-compaction
//! snapshot creation / rollback / cleanup. Extracted from
//! `session_manager.rs` to stay under the 1000-line file limit.

use std::sync::Arc;

use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::run_health::{RuntimeSnapshotManager, TranscriptOp};
use tokio::sync::RwLock;
use tracing::warn;

use super::SessionManager;

impl SessionManager {
    /// Persist the checkpoint after compaction to protect plan_state.
    ///
    /// Compaction replaces in-memory messages but does not modify the
    /// checkpoint directly. This method ensures the checkpoint (including
    /// plan_state) is written to durable storage immediately after
    /// compaction, so a subsequent crash does not lose the plan state.
    ///
    /// Follows the BootstrapProtection pattern: save a copy of the
    /// checkpoint before compaction is applied (the caller is responsible
    /// for calling this after `apply_compact_result`).
    pub async fn save_checkpoint_after_compact(&self, session_id: &str) {
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
        // Sync outbound_pending from ConversationSession so checkpoint
        // reflects the post-compaction state (design doc §数据流).
        {
            let conv_sessions = self.conversation_sessions.read().await;
            if let Some(cs) = conv_sessions.get(session_id) {
                let cs = cs.read().await;
                cp.outbound_pending = cs.get_pending_messages();
            }
        }
        // Sync transcript (boundary messages after compaction).
        // Design doc §设计原则: transcript is the single source of truth.
        {
            let conv_sessions = self.conversation_sessions.read().await;
            if let Some(cs) = conv_sessions.get(session_id) {
                let cs = cs.read().await;
                cp.transcript = cs.messages().to_vec();
            }
        }
        cp.touch();
        if let Err(e) = cm.save_raw(&cp).await {
            warn!(
                session_id = %session_id,
                "failed to persist checkpoint after compaction: {}",
                e
            );
        }
    }

    /// Save a pre-compaction snapshot of the session messages.
    ///
    /// Must be called **before** compaction begins so that a failed
    /// compaction can be rolled back via [`rollback_compaction`].
    pub async fn save_pre_compaction_snapshot(&self, session_id: &str) {
        // Ensure a snapshot manager exists for this session.
        {
            let mut mgrs = self.snapshot_managers.write().await;
            mgrs.entry(session_id.to_string())
                .or_insert_with(|| Arc::new(RwLock::new(RuntimeSnapshotManager::new())));
        }
        let conv_sessions = self.conversation_sessions.read().await;
        let Some(cs) = conv_sessions.get(session_id) else {
            warn!(
                session_id = %session_id,
                "save_pre_compaction_snapshot: session not found"
            );
            return;
        };
        let cs = cs.read().await;
        let mgrs = self.snapshot_managers.read().await;
        let Some(mgr) = mgrs.get(session_id) else {
            warn!(
                session_id = %session_id,
                "save_pre_compaction_snapshot: snapshot manager not found"
            );
            return;
        };
        let mut mgr = mgr.write().await;
        mgr.create_snapshot(cs.messages(), TranscriptOp::Rewrite);
    }

    /// Rollback a failed compaction by restoring the pre-compaction
    /// snapshot.
    ///
    /// Returns `true` if a snapshot existed and was restored;
    /// `false` if no snapshot was saved (caller should treat as
    /// an error).
    pub async fn rollback_compaction(&self, session_id: &str) -> bool {
        let mgrs = self.snapshot_managers.read().await;
        let Some(mgr) = mgrs.get(session_id) else {
            warn!(
                session_id = %session_id,
                "rollback_compaction: snapshot manager not found"
            );
            return false;
        };
        let mut mgr = mgr.write().await;
        let Some(messages) = mgr.rollback() else {
            return false;
        };
        drop(mgr);
        drop(mgrs);
        // Restore messages into the ConversationSession.
        let conv_sessions = self.conversation_sessions.read().await;
        let Some(cs) = conv_sessions.get(session_id) else {
            warn!(
                session_id = %session_id,
                "rollback_compaction: session not found"
            );
            return false;
        };
        let mut cs = cs.write().await;
        cs.replace_messages(messages);
        true
    }

    /// Clear the pre-compaction snapshot after a successful compaction.
    pub async fn clear_pre_compaction_snapshot(&self, session_id: &str) {
        let mgrs = self.snapshot_managers.read().await;
        if let Some(mgr) = mgrs.get(session_id) {
            let mut mgr = mgr.write().await;
            mgr.clear();
        }
    }
}
