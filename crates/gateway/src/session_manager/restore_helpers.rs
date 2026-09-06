//! Helpers for restoring sessions from checkpoints.
//!
//! Extracted from resolve.rs to keep file size under the 1000-line limit.

use super::{session_helpers, SessionManager};
use crate::Message;
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::SessionCheckpoint;
use closeclaw_session::run_health::TranscriptOp;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

type CmRef = closeclaw_session::checkpoint_manager::CheckpointManager<
    dyn closeclaw_session::persistence::PersistenceService,
>;

impl SessionManager {
    /// Rebuild a ConversationSession from a checkpoint and restore
    /// its state.
    ///
    /// This is the shared logic used by both the migrating-timeout
    /// restore path and the archived-restore path in `resolve()`.
    /// It:
    /// 1. Creates a new ConversationSession (or rebuilds prompt for
    ///    in-memory one)
    /// 2. Restores pending messages, system_appends, verbosity, etc.
    /// 3. Injects recovery notifications and tool failure results
    ///
    /// Returns `Ok(())` on success.
    pub(super) async fn rebuild_session_from_checkpoint(
        &self,
        session_id: &str,
        cp: &SessionCheckpoint,
        message: &Message,
    ) -> Result<(), closeclaw_common::processor::ProcessError> {
        let cm_arc = {
            let guard = self.checkpoint_manager.read().await;
            guard.as_ref().map(Arc::clone)
        };
        let cm = match cm_arc {
            Some(cm) => cm,
            None => return Ok(()),
        };

        self.ensure_or_create_conversation_session(session_id, cp, message, &cm)
            .await?;

        self.restore_checkpoint_state(session_id, cp).await;

        self.inject_recovery_notifications(session_id, cp).await;

        Ok(())
    }

    /// Create a new ConversationSession or rebuild prompt for an
    /// existing one.
    async fn ensure_or_create_conversation_session(
        &self,
        session_id: &str,
        cp: &SessionCheckpoint,
        message: &Message,
        cm: &CmRef,
    ) -> Result<(), closeclaw_common::processor::ProcessError> {
        let needs_conv = {
            let cs = self.conversation_sessions.read().await;
            !cs.contains_key(session_id)
        };
        if needs_conv {
            self.create_new_conversation_session(session_id, cp, message, cm)
                .await?;
        } else {
            info!(
                session_id = %session_id,
                event = "session_injection",
                trigger = "session_restore",
                "rebuilding prompt for restored session \
                 in memory"
            );
            self.rebuild_archived_session_prompt(session_id, cp, message)
                .await;
        }
        Ok(())
    }

    /// Create and fully configure a new ConversationSession from
    /// checkpoint data, then insert it into the sessions map.
    async fn create_new_conversation_session(
        &self,
        session_id: &str,
        cp: &SessionCheckpoint,
        message: &Message,
        cm: &CmRef,
    ) -> Result<(), closeclaw_common::processor::ProcessError> {
        let agent_id = cp.agent_id.clone().unwrap_or_else(|| message.to.clone());
        let workdir_path = session_helpers::compute_session_workdir(
            true,
            session_id,
            message,
            &self.workspace_dir,
            cm,
        )
        .await?;

        let mut conv_session =
            ConversationSession::new(session_id.to_string(), "default".to_string(), workdir_path)
                .with_system_prompt("")
                .with_reasoning_level(self.default_reasoning_level);
        self.apply_default_cache_break_thresholds(&mut conv_session);

        self.inject_session_deps(&mut conv_session, session_id, &agent_id)
            .await;

        {
            let mut cs = self.conversation_sessions.write().await;
            cs.insert(session_id.to_string(), Arc::new(RwLock::new(conv_session)));
        }
        Ok(())
    }

    /// Inject all required dependencies into a
    /// ConversationSession, then rebuild its prompt and apply
    /// session config.
    async fn inject_session_deps(
        &self,
        conv_session: &mut ConversationSession,
        session_id: &str,
        agent_id: &str,
    ) {
        self.wire_core_session_deps(conv_session, agent_id).await;
        self.rebuild_and_finalize_session(conv_session, session_id, agent_id)
            .await;
    }

    /// Wire core dependencies: shutdown handle, LLM caller,
    /// system prompt builder, prompt overrides, dynamic prompt
    /// builder, skill listing provider and agent skills.
    async fn wire_core_session_deps(&self, conv: &mut ConversationSession, agent_id: &str) {
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
        self.wire_skill_listing_deps(conv, agent_id).await;
    }

    /// Query bootstrap mode, rebuild the system prompt, inject
    /// snapshot meta and checkpoint storage, and apply session
    /// config.
    async fn rebuild_and_finalize_session(
        &self,
        conv: &mut ConversationSession,
        session_id: &str,
        agent_id: &str,
    ) {
        let bootstrap_mode = self
            .query_agent_bootstrap_mode(agent_id)
            .await
            .unwrap_or(BootstrapMode::Full);
        *conv = conv.clone().with_bootstrap_mode(bootstrap_mode);
        info!(
            session_id = %session_id,
            event = "session_injection",
            trigger = "session_restore",
            "full injection for restored session \
             (new ConversationSession)"
        );
        conv.rebuild_system_prompt(session_id, agent_id, Some(bootstrap_mode))
            .await;
        self.inject_snapshot_meta_store(session_id, conv).await;
        self.inject_checkpoint_storage(conv).await;
        if let Some(cfg) = self.get_session_config_for_agent(agent_id).await {
            conv.set_git_status(cfg.is_git_status_enabled);
        }
    }

    /// Restore pending messages, system_appends, verbosity, and
    /// transcript from checkpoint into the ConversationSession.
    async fn restore_checkpoint_state(&self, session_id: &str, cp: &SessionCheckpoint) {
        let cs = self.conversation_sessions.read().await;
        if let Some(cs) = cs.get(session_id) {
            let mut cs = cs.write().await;
            cs.restore_pending_messages(cp.outbound_pending.clone());
            cs.restore_system_appends(cp.system_appends.clone());
            cs.set_verbosity_level(cp.verbosity_level);
            if let Some(ref comm_config) = cp.communication_config {
                cs.set_communication_config(comm_config.clone());
            }
            Self::sync_plan_file_path_from_checkpoint(&mut cs, cp);
            if !cp.pending_messages.is_empty() {
                cs.apply_transcript_op(TranscriptOp::Rewrite, cp.pending_messages.clone());
            }
        }
    }

    /// Inject recovery notifications and tool failure results from
    /// checkpoint (set by SessionRecoveryService during startup).
    async fn inject_recovery_notifications(&self, session_id: &str, cp: &SessionCheckpoint) {
        let has_recovery =
            cp.recovery_notification.is_some() || !cp.pending_tool_failures.is_empty();
        if has_recovery {
            let cs = self.conversation_sessions.read().await;
            if let Some(cs) = cs.get(session_id) {
                let mut cs = cs.write().await;
                // Tool failure results are injected before the system notification
                // so the transcript ends with: tool_result, then system notification.
                // This matches the design doc: the LLM sees tool failure first, then
                // the recovery summary, consistent with normal tool failure flow.
                for failure in &cp.pending_tool_failures {
                    let tool_call_id = serde_json::from_str::<serde_json::Value>(failure)
                        .ok()
                        .and_then(|v| v.get("op_id")?.as_str().map(String::from))
                        .unwrap_or_else(|| "recovery".to_string());
                    cs.inject_tool_result(&tool_call_id, failure);
                }
                if let Some(ref notification) = cp.recovery_notification {
                    cs.inject_system_message(notification.clone());
                }
                info!(
                    session_id = %session_id,
                    "injected recovery notification and {} \
                     tool failure(s)",
                    cp.pending_tool_failures.len()
                );
            }
        }
    }
}
