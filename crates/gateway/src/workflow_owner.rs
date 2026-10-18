//! Workflow gateway entry points backed by
//! [`WorkflowControl`](closeclaw_common::WorkflowControl) (entry ②).
//!
//! Design: gateway README「入站路径 · 路由决策」and
//! workflow-tools.md「进入模式触发」.
//!
//! - Owner non-slash messages notify the workflow runtime before routing to
//!   the session (a resolvable block is lifted by the engine). The message
//!   itself is never consumed here.
//! - `/workflow` is Owner-only: non-owner senders are hard-intercepted at
//!   the gateway layer with a permission-denied reply, never reaching the
//!   SlashDispatcher.

use super::{Gateway, HandleResult};

#[cfg(test)]
mod tests;

impl Gateway {
    /// Notify the workflow runtime of an owner non-slash message before
    /// routing (`closeclaw_common::WorkflowControl::on_owner_message`).
    ///
    /// The sender counts as the session owner when it matches the
    /// checkpoint sender id (`SessionManager::get_sender_id`). No-op when
    /// the sender is not the owner or no control is wired. Engine errors
    /// are logged and never block routing.
    pub(crate) async fn notify_workflow_owner_message(
        &self,
        session_id: &str,
        sender_id: Option<&str>,
    ) {
        let Some(sender) = sender_id else {
            return;
        };
        let owner_id = self.session_manager.get_sender_id(session_id).await;
        if owner_id.as_deref() != Some(sender) {
            return;
        }
        let Some(control) = self.session_manager.get_workflow_control().await else {
            return;
        };
        if let Err(e) = control.on_owner_message(session_id).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "workflow on_owner_message failed; message continues routing"
            );
        }
    }

    /// Hard-intercept a non-owner `/workflow` command at the gateway layer.
    ///
    /// Returns `Some(HandleResult::SlashHandled)` after replying
    /// "权限不足" — the command never reaches the SlashDispatcher.
    /// Returns `None` for owner senders and non-`/workflow` content so
    /// routing continues.
    pub(crate) async fn try_intercept_workflow_command(
        &self,
        content: &str,
        sender_id: Option<&str>,
        peer_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        let (cmd, _) = super::slash_permission::parse_slash(content)?;
        if cmd != "workflow" || sender_id.is_some_and(closeclaw_permission::is_owner) {
            return None;
        }
        tracing::warn!(
            sender_id = ?sender_id,
            "non-owner attempted /workflow; rejected at gateway layer"
        );
        if let Err(e) = self
            .send_outbound_simplified(peer_id, channel, "权限不足：该指令仅限 Owner 使用")
            .await
        {
            tracing::warn!(error = %e, "failed to send /workflow rejection reply");
        }
        Some(HandleResult::SlashHandled)
    }
}
