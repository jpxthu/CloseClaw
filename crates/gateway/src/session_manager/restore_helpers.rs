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

impl SessionManager {
    /// Rebuild a ConversationSession from a checkpoint and restore its state.
    ///
    /// This is the shared logic used by both the migrating-timeout restore
    /// path and the archived-restore path in `resolve()`. It:
    /// 1. Creates a new ConversationSession (or rebuilds prompt for in-memory one)
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

        // Ensure ConversationSession exists
        let needs_conv = {
            let cs = self.conversation_sessions.read().await;
            !cs.contains_key(session_id)
        };
        if needs_conv {
            let agent_id = cp.agent_id.clone().unwrap_or_else(|| message.to.clone());
            let workdir_path = session_helpers::compute_session_workdir(
                true,
                session_id,
                message,
                &self.workspace_dir,
                cm.as_ref(),
            )
            .await?;

            let mut conv_session = ConversationSession::new(
                session_id.to_string(),
                "default".to_string(),
                workdir_path,
            )
            .with_system_prompt("")
            .with_reasoning_level(self.default_reasoning_level);
            self.apply_default_cache_break_thresholds(&mut conv_session);
            // Wire shutdown handle for busy-count tracking.
            if let Some(sh) = self.get_shutdown_handle().await {
                conv_session.set_shutdown_handle(sh);
            }
            // Inject LLM caller and system prompt builder for delegation.
            let agent_hooks = self
                .get_agent_config(&agent_id)
                .await
                .map(|c| c.hooks)
                .unwrap_or_default();
            if let Some(caller) = self.get_llm_caller().await {
                conv_session.set_llm_caller(caller.clone());
                conv_session.init_health_checker(caller, agent_hooks);
            }
            if let Some(builder) = self.get_system_prompt_builder().await {
                conv_session.set_system_prompt_builder(builder);
            }
            conv_session.set_prompt_overrides(self.get_prompt_overrides().await);
            // Inject dynamic prompt builder for per-request
            // dynamic-layer injection (ChannelContext, etc.).
            if let Some(dpb) = self.get_dynamic_prompt_builder().await {
                conv_session.set_dynamic_prompt_builder(dpb);
            }
            // Inject skill listing provider and agent skills.
            self.wire_skill_listing_deps(&mut conv_session, &agent_id)
                .await;
            // Query bootstrap mode from AgentRegistry and cache.
            let bootstrap_mode = self
                .query_agent_bootstrap_mode(&agent_id)
                .await
                .unwrap_or(BootstrapMode::Full);
            conv_session = conv_session.with_bootstrap_mode(bootstrap_mode);
            // Build initial system prompt via session's own builder.
            info!(
                session_id = %session_id,
                event = "session_injection",
                trigger = "session_restore",
                "full injection for restored session (new ConversationSession)"
            );
            conv_session
                .rebuild_system_prompt(session_id, &agent_id, Some(bootstrap_mode))
                .await;
            // Inject snapshot meta store for persistence.
            self.inject_snapshot_meta_store(session_id, &mut conv_session)
                .await;
            // Inject checkpoint storage for pending-operation persistence.
            self.inject_checkpoint_storage(&mut conv_session).await;
            // Apply session config (git_status switch).
            if let Some(cfg) = self.get_session_config_for_agent(&agent_id).await {
                conv_session.set_git_status(cfg.is_git_status_enabled);
            }
            {
                let mut cs = self.conversation_sessions.write().await;
                cs.insert(session_id.to_string(), Arc::new(RwLock::new(conv_session)));
            }
        } else {
            info!(
                session_id = %session_id,
                event = "session_injection",
                trigger = "session_restore",
                "rebuilding prompt for restored session in memory"
            );
            self.rebuild_archived_session_prompt(session_id, cp, message)
                .await;
        }

        // Restore pending messages, system_appends, verbosity_level,
        // and communication_config from checkpoint.
        // NOTE: system_appends must be restored AFTER rebuild_system_prompt
        // so that user appends layer on top of the rebuilt prompt.
        {
            let cs = self.conversation_sessions.read().await;
            if let Some(cs) = cs.get(session_id) {
                let mut cs = cs.write().await;
                cs.restore_pending_messages(cp.outbound_pending.clone());
                cs.restore_system_appends(cp.system_appends.clone());
                cs.set_verbosity_level(cp.verbosity_level);
                // Restore communication config for spawned sessions.
                if let Some(ref comm_config) = cp.communication_config {
                    cs.set_communication_config(comm_config.clone());
                }
                Self::sync_plan_file_path_from_checkpoint(&mut cs, cp);
                // Restore transcript from checkpoint ("transcript is the
                // single source of truth" per design doc).
                if !cp.pending_messages.is_empty() {
                    cs.apply_transcript_op(TranscriptOp::Rewrite, cp.pending_messages.clone());
                }
            }
        }

        // Inject recovery notifications and tool failure results
        // from checkpoint (set by SessionRecoveryService during startup).
        if let Some(ref notification) = cp.recovery_notification {
            let cs = self.conversation_sessions.read().await;
            if let Some(cs) = cs.get(session_id) {
                let mut cs = cs.write().await;
                cs.inject_system_message(notification.clone());
                for failure in &cp.pending_tool_failures {
                    // Extract op_id from the JSON failure string to use
                    // as tool_call_id.  Falls back to "recovery" if parsing
                    // fails (defensive — the JSON is built by the recovery
                    // service and always contains op_id).
                    let tool_call_id = serde_json::from_str::<serde_json::Value>(failure)
                        .ok()
                        .and_then(|v| v.get("op_id")?.as_str().map(String::from))
                        .unwrap_or_else(|| "recovery".to_string());
                    cs.inject_tool_result(&tool_call_id, failure);
                }
                info!(
                    session_id = %session_id,
                    "injected recovery notification and {} tool failure(s)",
                    cp.pending_tool_failures.len()
                );
            }
        }

        Ok(())
    }
}
