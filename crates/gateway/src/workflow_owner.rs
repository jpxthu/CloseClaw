//! Workflow owner response handling for blocked workflows (Step 1.6).
//!
//! When a workflow is in blocked state, owner messages are intercepted
//! and routed to the engine for resolve or terminate.

use super::Gateway;
use super::HandleResult;
use closeclaw_session::workflow_recovery::DEFINITION_CHANGED_PAUSE_REASON;

#[cfg(test)]
mod tests;

impl Gateway {
    /// Handle owner response ("恢复"/"终止") to a blocked workflow (Step 1.6).
    pub(crate) async fn try_handle_workflow_owner_response(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
    ) -> Option<HandleResult> {
        let action = self
            .resolve_owner_action(session_id, sender_id, content)
            .await?;
        self.apply_owner_action(session_id, &action).await?;
        self.persist_and_confirm(session_id, &action).await;
        Some(HandleResult::SlashHandled)
    }

    /// Apply the resolved owner action (resolve/terminate/rejected_resolve) to the session.
    async fn apply_owner_action(&self, session_id: &str, action: &str) -> Option<()> {
        let cs = self
            .session_manager
            .get_conversation_session(session_id)
            .await?;
        let mut cs_write = cs.write().await;
        match action {
            "resolve" => {
                Self::apply_resolve_action(&mut cs_write);
            }
            "terminate" => {
                Self::apply_terminate_action(&mut cs_write);
            }
            "rejected_resolve" => {
                Self::apply_rejected_resolve_action(&mut cs_write);
            }
            _ => return None,
        }
        Some(())
    }

    /// Check if the session has a blocked workflow and the sender is the owner.
    /// Returns the resolved action string ("resolve" / "terminate" / "rejected_resolve") or None.
    ///
    /// When the workflow is blocked due to a definition version change
    /// (`paused_reason == DEFINITION_CHANGED_PAUSE_REASON`), "resolve" is
    /// rejected because the design doc mandates that such pauses can only
    /// be terminated.
    async fn resolve_owner_action(
        &self,
        session_id: &str,
        sender_id: Option<&str>,
        content: &str,
    ) -> Option<String> {
        let cs = self
            .session_manager
            .get_conversation_session(session_id)
            .await?;
        // Single read-lock: check blocked + read paused_reason in one snapshot.
        let cs_read = cs.read().await;
        let is_blocked = cs_read.is_workflow_blocked();
        if !is_blocked {
            return None;
        }
        // Clone to release the lock before the owner check and content matching.
        let paused_reason = cs_read
            .workflow_run()
            .map(|r| r.paused_reason.clone())
            .unwrap_or_default();
        drop(cs_read);
        let owner_id = self.session_manager.get_sender_id(session_id).await;
        if sender_id.is_none_or(|sid| owner_id.as_ref().is_none_or(|o| o != sid)) {
            return None;
        }
        let trimmed = content.trim();
        if trimmed == "恢复" || trimmed.eq_ignore_ascii_case("resolve") {
            // Guard: definition-changed pauses can only be terminated.
            if paused_reason == DEFINITION_CHANGED_PAUSE_REASON {
                Some("rejected_resolve".to_string())
            } else {
                Some("resolve".to_string())
            }
        } else if trimmed == "终止" || trimmed.eq_ignore_ascii_case("terminate") {
            Some("terminate".to_string())
        } else {
            None
        }
    }

    /// Apply the resolve action: restore workflow messages and inject verification.
    fn apply_resolve_action(cs: &mut closeclaw_session::llm_session::ConversationSession) {
        cs.remove_workflow_verify_messages();
        if let Some(ref mut handler) = cs.workflow_handler_mut() {
            handler.on_owner_resolve();
        }
        // Capture snapshot AFTER on_owner_resolve() so pending_verify=0 and
        // phase=Verifying are preserved (not overwritten by a stale snapshot).
        if let Some(handler) = cs.workflow_handler() {
            let run_snapshot = handler.run().clone();
            let definition_snapshot = handler.definition().clone();
            let current_step = handler.run().current_step;
            if let Some(step) = definition_snapshot.steps.get(current_step) {
                let allow_blocked = step
                    .allow_blocked
                    .unwrap_or(definition_snapshot.allow_blocked);
                let verify_msg =
                    closeclaw_workflow::definition::build_verify_message(step, allow_blocked);
                cs.inject_workflow_message(&verify_msg);
            }
            cs.set_workflow_run(Some(run_snapshot));
        }
    }

    /// Apply the terminate action: clear all workflow state.
    fn apply_terminate_action(cs: &mut closeclaw_session::llm_session::ConversationSession) {
        cs.remove_workflow_context_from_appends();
        cs.remove_workflow_messages();
        if let Some(ref mut handler) = cs.workflow_handler_mut() {
            handler.on_owner_terminate();
        }
        cs.clear_workflow_run();
    }

    /// Apply the rejected-resolve action: workflow state is untouched.
    /// An informational message is injected so the owner learns that
    /// definition-change pauses can only be terminated.
    fn apply_rejected_resolve_action(cs: &mut closeclaw_session::llm_session::ConversationSession) {
        // \u{200b}: zero-width space after \n preserves the line break when
        // certain IM platforms (e.g. Feishu) strip trailing whitespace.
        cs.inject_workflow_message(
            "⚠️ 该暂停因定义版本变更产生，当前步骤在最新定义中已不存在。\n\u{200b}仅可回复「终止」结束工作流，无法解除暂停。",
        );
    }

    /// Persist the workflow state after owner response and send confirmation.
    async fn persist_and_confirm(&self, session_id: &str, action: &str) {
        // rejected_resolve does not change workflow state; skip persistence entirely.
        if action == "rejected_resolve" {
            return;
        }
        let run_to_persist = match self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            Some(c) => c.read().await.workflow_run().cloned(),
            None => None,
        };
        if let Err(e) = self
            .session_manager
            .set_workflow_run(session_id, run_to_persist)
            .await
        {
            tracing::warn!(session_id = %session_id, error = %e, "failed to persist workflow state after owner response");
        }
        if let Some(peer_id) = self.session_manager.get_sender_id(session_id).await {
            let Some(channel) = self.session_manager.session_channel(session_id).await else {
                return;
            };
            let msg = match action {
                "resolve" => "✅ 已恢复工作流执行。",
                "terminate" => "🛑 已终止工作流。",
                _ => unreachable!(),
            };
            if let Err(e) = self.send_outbound_simplified(&peer_id, &channel, msg).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to send workflow owner response confirmation"
                );
            }
        }
    }
}
