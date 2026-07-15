//! Graceful stop helpers for [`SessionManager`].
//!
//! Extracted from `stop.rs` to keep each `impl` block within the
//! 100-line limit required by CONTRIBUTING.md.

use std::sync::Arc;
use std::time::Duration;

use closeclaw_session::llm_session::session_handles::{GracefulStopProgress, GracefulStopResult};
use closeclaw_session::persistence::{
    PendingOperation, PersistenceError, PersistenceService, SessionCheckpoint, SessionStatus,
};

use super::stop::GracefulStopOutcome;
use super::SessionManager;

// ── graceful timeout logic ─────────────────────────────────────────────

impl SessionManager {
    /// Graceful wait with a hard timeout, delegated to
    /// [`ConversationSession::graceful_stop`].
    ///
    /// Returns `(pending_ops, outcome)` where:
    /// - `pending_ops` — tool calls extracted after streaming ended
    ///   (only meaningful for `Completed`)
    /// - `outcome` — [`GracefulStopOutcome`] indicating whether the
    ///   stop completed, timed out (with progress info), or was
    ///   interrupted by forceful escalation
    pub(super) async fn graceful_wait(
        &self,
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        session_id: &str,
        timeout: Duration,
        progress_tx: Option<tokio::sync::mpsc::Sender<GracefulStopProgress>>,
    ) -> (Vec<PendingOperation>, GracefulStopOutcome) {
        let (internal_tx, mut progress_rx) = tokio::sync::mpsc::channel(4);
        let result = cs
            .read()
            .await
            .graceful_stop(timeout, Some(internal_tx))
            .await;

        // Forward progress events to the caller's progress channel
        // (if any), log each event, and capture the last one for
        // the TimedOut outcome.
        let mut last_progress: Option<GracefulStopProgress> = None;
        while let Ok(progress) = progress_rx.try_recv() {
            tracing::info!(
                session_id = %session_id,
                remaining = progress.remaining,
                "graceful_wait: timeout progress — items still in flight"
            );
            last_progress = Some(progress.clone());
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(progress).await;
            }
        }

        match result {
            GracefulStopResult::Completed => {
                let pending_ops = cs.read().await.extract_pending_tool_calls();
                (pending_ops, GracefulStopOutcome::Completed)
            }
            GracefulStopResult::Interrupted => (Vec::new(), GracefulStopOutcome::Interrupted),
            GracefulStopResult::TimedOut => {
                let progress = last_progress.unwrap_or(GracefulStopProgress {
                    waiting_items: Vec::new(),
                    remaining: 0,
                });
                (
                    Vec::new(),
                    GracefulStopOutcome::TimedOut {
                        waiting_items: progress.waiting_items,
                        remaining: progress.remaining,
                    },
                )
            }
        }
    }
}

// ── checkpoint persistence ─────────────────────────────────────────────

impl SessionManager {
    /// Persist a session checkpoint with optional pending operations.
    /// Non-empty `pending_ops` (forceful shutdown) are recorded for recovery.
    pub(super) async fn persist_checkpoint_with_pending(
        &self,
        session_id: &str,
        pending_ops: Vec<PendingOperation>,
    ) -> Result<(), PersistenceError> {
        let storage_arc = {
            let guard = self.checkpoint_manager.read().await;
            match guard.as_ref() {
                Some(cm) => std::sync::Arc::clone(cm.storage_arc()),
                None => return Ok(()),
            }
        };

        let mut cp = self.build_checkpoint(session_id, &storage_arc).await?;

        // Record pending operations from forceful shutdown.
        if !pending_ops.is_empty() {
            cp.pending_operations = pending_ops;
        }

        storage_arc.save_checkpoint(&cp).await
    }

    /// Build a `SessionCheckpoint` for the given session, loading or creating
    /// as needed and syncing metadata from `ConversationSession`.
    async fn build_checkpoint(
        &self,
        session_id: &str,
        storage: &Arc<dyn PersistenceService>,
    ) -> Result<SessionCheckpoint, PersistenceError> {
        let (agent_id, channel) = self.session_routing_info(session_id).await;
        let pending = self.pending_messages_for(session_id).await;

        let mut cp =
            load_or_create_checkpoint(storage, session_id, agent_id, channel, pending).await?;

        sync_conversation_metadata(&self.conversation_sessions, session_id, &mut cp).await;
        Ok(cp)
    }

    /// Look up routing info (agent_id, channel) for a session.
    async fn session_routing_info(&self, session_id: &str) -> (Option<String>, Option<String>) {
        let sessions = self.sessions.read().await;
        match sessions.get(session_id) {
            Some(s) => (Some(s.agent_id.clone()), Some(s.channel.clone())),
            None => (None, None),
        }
    }

    /// Collect pending messages from the conversation session.
    async fn pending_messages_for(
        &self,
        session_id: &str,
    ) -> Vec<closeclaw_session::persistence::PendingMessage> {
        let conv = self.conversation_sessions.read().await;
        match conv.get(session_id) {
            Some(cs) => {
                let guard = cs.read().await;
                guard.get_pending_messages()
            }
            None => Vec::new(),
        }
    }
}

// ── checkpoint helpers ────────────────────────────────────────────────

/// Load an existing checkpoint or create a new one, applying the given
/// routing info and pending messages.
async fn load_or_create_checkpoint(
    storage: &Arc<dyn PersistenceService>,
    session_id: &str,
    agent_id: Option<String>,
    channel: Option<String>,
    pending: Vec<closeclaw_session::persistence::PendingMessage>,
) -> Result<SessionCheckpoint, PersistenceError> {
    match storage.load_checkpoint(session_id).await? {
        Some(mut cp) => {
            cp.status = SessionStatus::Active;
            cp.platform = channel;
            cp.agent_id = agent_id;
            cp.outbound_pending = pending;
            Ok(cp)
        }
        None => {
            let mut cp =
                SessionCheckpoint::new(session_id.to_string()).with_status(SessionStatus::Active);
            if let Some(ch) = channel {
                cp = cp.with_platform(ch);
            }
            if let Some(aid) = agent_id {
                cp = cp.with_agent_id(aid);
            }
            Ok(cp.with_outbound_pending(pending))
        }
    }
}

/// Sync `system_appends` and `verbosity_level` from the conversation session.
async fn sync_conversation_metadata(
    conv: &tokio::sync::RwLock<
        std::collections::HashMap<
            String,
            Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        >,
    >,
    session_id: &str,
    cp: &mut SessionCheckpoint,
) {
    let conv_guard = conv.read().await;
    if let Some(cs) = conv_guard.get(session_id) {
        let guard = cs.read().await;
        cp.system_appends = guard.user_system_appends().to_vec();
        cp.verbosity_level = guard.verbosity_level();
    }
}
