//! Startup recovery notification injection.
//!
//! Creates `ConversationSession`s for dirty sessions at daemon startup and
//! injects recovery notifications + tool failure results from checkpoints.
//! This ensures recovery data is available immediately, without waiting for
//! an inbound message to trigger `resolve()`.

use super::SessionManager;
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint};
use closeclaw_session::run_health::TranscriptOp;
use closeclaw_session::workspace;
use std::sync::Arc;
use tracing::{info, warn};

// --- Orchestration ---------------------------------------------------------

impl SessionManager {
    /// Inject recovery notifications into `ConversationSession`s at startup.
    ///
    /// For each dirty session (one with `pending_operations` from a previous
    /// crash), creates a `ConversationSession` and injects the recovery
    /// notification and tool failure results from the checkpoint.
    ///
    /// After injection, the checkpoint's recovery data is cleared to prevent
    /// double-injection if the session is later archived and restored via
    /// `resolve()`.
    pub async fn inject_startup_recovery_notifications(&self, dirty_sessions: &[String]) {
        let cm_arc = match self.get_checkpoint_manager_arc().await {
            Some(cm) => cm,
            None => {
                warn!(
                    "checkpoint_manager not available, \
                     skipping startup recovery injection"
                );
                return;
            }
        };

        for session_id in dirty_sessions {
            self.inject_one_session(session_id, &cm_arc).await;
        }
    }

    /// Get an `Arc` clone of the internal `CheckpointManager`.
    async fn get_checkpoint_manager_arc(
        &self,
    ) -> Option<Arc<CheckpointManager<dyn PersistenceService>>> {
        let guard = self.checkpoint_manager.read().await;
        guard.as_ref().map(Arc::clone)
    }

    /// Process a single dirty session: load checkpoint, create
    /// `ConversationSession`, inject recovery data, persist.
    async fn inject_one_session(
        &self,
        session_id: &str,
        cm: &Arc<CheckpointManager<dyn PersistenceService>>,
    ) {
        // Skip if ConversationSession already exists (concurrent resolve).
        if self
            .conversation_sessions
            .read()
            .await
            .contains_key(session_id)
        {
            info!(
                session_id = %session_id,
                "ConversationSession already exists, skipping"
            );
            return;
        }

        let cp = match cm.load(session_id).await {
            Ok(Some(cp)) => cp,
            _ => {
                warn!(
                    session_id = %session_id,
                    "checkpoint not found or failed to load, skipping"
                );
                return;
            }
        };

        if cp.recovery_notification.is_none() && cp.pending_tool_failures.is_empty() {
            info!(
                session_id = %session_id,
                "no recovery data in checkpoint, skipping"
            );
            return;
        }

        let agent_id = cp.agent_id.as_deref().unwrap_or(session_id);
        let workdir = self.compute_workdir_for_agent(agent_id);
        let mut conv = self.build_conv_session(session_id, agent_id, workdir).await;
        self.wire_conv_dependencies(&mut conv, agent_id).await;
        self.restore_transcript_from_checkpoint(&mut conv, &cp);
        self.inject_recovery_data(&mut conv, &cp);
        self.restore_conv_metadata(&mut conv, &cp);
        self.insert_conv_session(session_id, conv).await;
        self.ensure_session_entry(session_id, agent_id).await;
        self.clear_checkpoint_recovery(cm, cp, session_id).await;

        info!(
            session_id = %session_id,
            "recovery notification injected at startup"
        );
    }
}

// --- ConversationSession building ------------------------------------------

impl SessionManager {
    /// Compute the workspace directory for an agent.
    fn compute_workdir_for_agent(&self, agent_id: &str) -> std::path::PathBuf {
        if let Some(ref wd) = self.workspace_dir {
            workspace::ensure_workspace_dir(wd, agent_id, agent_id)
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        } else {
            std::path::PathBuf::from("/tmp")
        }
    }

    /// Build a minimal `ConversationSession` for recovery injection.
    async fn build_conv_session(
        &self,
        session_id: &str,
        agent_id: &str,
        workdir: std::path::PathBuf,
    ) -> ConversationSession {
        let bootstrap_mode = self
            .query_agent_bootstrap_mode(agent_id)
            .await
            .unwrap_or(BootstrapMode::Full);
        let mut conv =
            ConversationSession::new(session_id.to_string(), "default".to_string(), workdir)
                .with_system_prompt("")
                .with_reasoning_level(self.default_reasoning_level)
                .with_bootstrap_mode(bootstrap_mode);
        conv.rebuild_system_prompt(session_id, agent_id, Some(bootstrap_mode))
            .await;
        self.inject_snapshot_meta_store(session_id, &mut conv).await;
        self.inject_checkpoint_storage(&mut conv).await;
        conv
    }

    /// Wire injectable dependencies into a `ConversationSession`.
    async fn wire_conv_dependencies(&self, conv: &mut ConversationSession, agent_id: &str) {
        if let Some(sh) = self.get_shutdown_handle().await {
            conv.set_shutdown_handle(sh);
        }
        let agent_hooks = self
            .get_agent_config(agent_id)
            .await
            .map(|c| c.hooks)
            .unwrap_or_default();
        if let Some(caller) = self.get_llm_caller().await {
            conv.set_llm_caller(caller.clone());
            conv.init_health_checker(caller, agent_hooks);
        }
        if let Some(builder) = self.get_system_prompt_builder().await {
            conv.set_system_prompt_builder(builder);
        }
        conv.set_prompt_overrides(self.get_prompt_overrides().await);
        if let Some(dpb) = self.get_dynamic_prompt_builder().await {
            conv.set_dynamic_prompt_builder(dpb);
        }
    }
}

// --- Data injection --------------------------------------------------------

impl SessionManager {
    /// Restore transcript messages from checkpoint.
    fn restore_transcript_from_checkpoint(
        &self,
        conv: &mut ConversationSession,
        cp: &SessionCheckpoint,
    ) {
        if !cp.pending_messages.is_empty() {
            conv.apply_transcript_op(TranscriptOp::Rewrite, cp.pending_messages.clone());
        }
    }

    /// Inject recovery notification and tool failure results.
    fn inject_recovery_data(&self, conv: &mut ConversationSession, cp: &SessionCheckpoint) {
        if let Some(ref notification) = cp.recovery_notification {
            conv.inject_system_message(notification.clone());
        }
        for failure in &cp.pending_tool_failures {
            let tool_call_id = serde_json::from_str::<serde_json::Value>(failure)
                .ok()
                .and_then(|v| v.get("op_id")?.as_str().map(String::from))
                .unwrap_or_else(|| "recovery".to_string());
            conv.inject_tool_result(&tool_call_id, failure);
        }
    }

    /// Restore pending messages, system_appends, verbosity, and
    /// communication config from checkpoint.
    fn restore_conv_metadata(&self, conv: &mut ConversationSession, cp: &SessionCheckpoint) {
        conv.restore_pending_messages(cp.outbound_pending.clone());
        conv.restore_system_appends(cp.system_appends.clone());
        conv.set_verbosity_level(cp.verbosity_level);
        if let Some(ref comm_config) = cp.communication_config {
            conv.set_communication_config(comm_config.clone());
        }
    }
}

// --- Session map management ------------------------------------------------

impl SessionManager {
    /// Insert the `ConversationSession` into the sessions map.
    async fn insert_conv_session(&self, session_id: &str, conv: ConversationSession) {
        let mut cs = self.conversation_sessions.write().await;
        cs.insert(
            session_id.to_string(),
            Arc::new(tokio::sync::RwLock::new(conv)),
        );
    }

    /// Ensure a `Session` entry exists in the sessions map.
    async fn ensure_session_entry(&self, session_id: &str, agent_id: &str) {
        let mut sessions = self.sessions.write().await;
        if !sessions.contains_key(session_id) {
            sessions.insert(
                session_id.to_string(),
                super::session_helpers::create_session_from_checkpoint(session_id, agent_id),
            );
        }
    }

    /// Clear recovery data from checkpoint after successful injection.
    async fn clear_checkpoint_recovery(
        &self,
        cm: &Arc<CheckpointManager<dyn PersistenceService>>,
        mut cp: SessionCheckpoint,
        session_id: &str,
    ) {
        cp.recovery_notification = None;
        cp.pending_tool_failures.clear();
        if let Err(e) = cm.save_raw(&cp).await {
            warn!(
                session_id = %session_id,
                error = %e,
                "failed to save checkpoint after clearing recovery data"
            );
        }
    }
}
