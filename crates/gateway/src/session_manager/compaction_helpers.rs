//! Compaction snapshot management for [`SessionManager`].
//!
//! Handles checkpoint persistence after compaction and pre-compaction
//! snapshot creation / rollback / cleanup. Extracted from
//! `session_manager.rs` to stay under the 1000-line file limit.

use std::sync::Arc;

use closeclaw_session::llm_session::ChatSession;
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
                cp.pending_messages = cs.messages().to_vec();
            }
        }
        // Sync system_appends from ConversationSession so checkpoint
        // reflects any in-memory additions (e.g. workflow context injection)
        // that happened since the last checkpoint save.
        {
            let conv_sessions = self.conversation_sessions.read().await;
            if let Some(cs) = conv_sessions.get(session_id) {
                let cs = cs.read().await;
                cp.system_appends = cs.user_system_appends().to_vec();
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
        // Re-inject workflow context if missing after compaction.
        // Must happen after checkpoint save so the definition reload reads
        // the fresh checkpoint state.
        self.reinject_workflow_context_after_compact(session_id)
            .await;
    }

    /// Save a pre-compaction snapshot of the session messages.
    ///
    /// Must be called **before** compaction begins so that a failed
    /// compaction can be rolled back via [`rollback_compaction`].
    ///
    /// Delegates to the session's internal snapshot manager — no
    /// external snapshot management needed.
    ///
    /// Returns the snapshot id if a snapshot was created, or `None`
    /// if the session was not found.
    pub async fn save_pre_compaction_snapshot(&self, session_id: &str) -> Option<String> {
        let conv_sessions = self.conversation_sessions.read().await;
        let Some(cs) = conv_sessions.get(session_id) else {
            warn!(
                session_id = %session_id,
                "save_pre_compaction_snapshot: session not found"
            );
            return None;
        };
        let mut cs = cs.write().await;
        cs.snapshot_current_state(
            closeclaw_session::run_health::TranscriptOp::Rewrite,
            "pre-compaction",
        )
    }

    /// Rollback a failed compaction by restoring the pre-compaction
    /// snapshot.
    ///
    /// Returns `true` if a snapshot existed and was restored;
    /// `false` if no snapshot was saved (caller should treat as
    /// an error).
    pub async fn rollback_compaction(&self, session_id: &str) -> bool {
        let conv_sessions = self.conversation_sessions.read().await;
        let Some(cs) = conv_sessions.get(session_id) else {
            warn!(
                session_id = %session_id,
                "rollback_compaction: session not found"
            );
            return false;
        };
        let mut cs = cs.write().await;
        cs.rollback_transcript().is_some()
    }

    /// Mark the pre-compaction snapshot as complete after a
    /// successful compaction.
    ///
    /// The snapshot is retained in the bounded queue for potential
    /// future rollback, rather than being cleared. Old snapshots are
    /// evicted automatically when the queue exceeds `MAX_SNAPSHOTS`.
    pub async fn complete_pre_compaction_snapshot(&self, session_id: &str, snapshot_id: &str) {
        let conv_sessions = self.conversation_sessions.read().await;
        if let Some(cs) = conv_sessions.get(session_id) {
            let mut cs = cs.write().await;
            cs.mark_complete_snapshot(snapshot_id);
        }
    }

    /// Create a partial-rewrite snapshot for a session.
    ///
    /// Called before `/system` modifications (add or clear) to capture
    /// the current transcript state. Returns the snapshot id if a
    /// snapshot was created, or `None` if the session was not found.
    pub async fn create_partial_rewrite_snapshot(&self, session_id: &str) -> Option<String> {
        let conv_sessions = self.conversation_sessions.read().await;
        let Some(cs) = conv_sessions.get(session_id) else {
            warn!(
                session_id = %session_id,
                "create_partial_rewrite_snapshot: session not found"
            );
            return None;
        };
        let mut cs = cs.write().await;
        cs.snapshot_current_state(
            closeclaw_session::run_health::TranscriptOp::PartialRewrite,
            "system-prompt-update",
        )
    }

    /// Returns the snapshot count for a session, or `None` if no
    /// snapshot manager exists for that session.
    #[cfg(test)]
    pub async fn snapshot_count_for(&self, session_id: &str) -> Option<usize> {
        let conv_sessions = self.conversation_sessions.read().await;
        let cs = conv_sessions.get(session_id)?;
        let cs = cs.read().await;
        cs.snapshot_count()
    }

    /// Re-inject workflow context into system_appends after compaction.
    ///
    /// After compaction completes, system_appends may have been cleared.
    /// If an active workflow exists in the checkpoint (phase != Complete),
    /// reload the definition via three-level lookup and re-inject the
    /// workflow context so the agent maintains workflow awareness.
    ///
    /// Called from [`save_checkpoint_after_compact`] as part of the
    /// post-compaction pipeline.
    pub(crate) async fn reinject_workflow_context_after_compact(&self, session_id: &str) {
        // Load checkpoint to check for active workflow.
        let cp = {
            let cm_guard = self.checkpoint_manager.read().await;
            let Some(cm) = cm_guard.as_ref() else {
                return;
            };
            let cm = std::sync::Arc::clone(cm);
            match cm.load(session_id).await {
                Ok(Some(cp)) => cp,
                _ => return,
            }
        };

        let Some(ref run) = cp.workflow_run else {
            return;
        };

        // Skip if workflow is already complete.
        if run.phase == closeclaw_workflow::run::Phase::Complete {
            return;
        }

        // Check if workflow context already exists in system_appends.
        if closeclaw_workflow::context_append::has_workflow_context(&cp.system_appends) {
            return;
        }

        // Workflow context is missing — reload the definition and re-inject.
        let definition_name = &run.definition_name;
        if definition_name.is_empty() {
            tracing::warn!(
                session_id = %session_id,
                "workflow_run exists but definition_name is empty, cannot re-inject context"
            );
            return;
        }

        // Resolve agent workspace for three-level lookup.
        let agent_ws = match cp.agent_id {
            Some(ref agent_id) => self.query_agent_workspace(agent_id.as_str()).await,
            None => None,
        };
        let dot_closeclaw = dirs::home_dir().map(|h| h.join(".closeclaw"));

        let workflow = match closeclaw_workflow::definition_loader::WorkflowDefinitionLoader::load(
            definition_name,
            agent_ws.as_deref(),
            dot_closeclaw.as_deref(),
        ) {
            Ok(wf) => wf,
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    definition_name = %definition_name,
                    error = %e,
                    "failed to reload workflow definition for post-compaction re-injection"
                );
                return;
            }
        };

        let context = closeclaw_workflow::context_append::build_workflow_context_append(&workflow);

        // Inject into ConversationSession's system_appends.
        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs = cs.write().await;
            cs.add_system_append(context);
        }
    }
}
