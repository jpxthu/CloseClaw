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
//!
//! # Owner 口径（双入口一致性）
//!
//! All three workflow entries use the **checkpoint sender id**
//! (`SessionManager::get_sender_id`, bound at session creation from the
//! raw platform sender — `session_manager/resolve.rs`:
//! `cp.sender_id = message.from`) as the Owner criterion:
//!
//! - `/workflow` hard intercept: sender must equal the checkpoint owner
//!   (below, `try_intercept_workflow_command`).
//! - Entry ② owner notification: sender must equal the checkpoint owner
//!   (`notify_workflow_owner_message`), same comparison as entry ③ in
//!   `slash_executor_helpers.rs::terminate_workflow_after_stop`.
//!
//! Step 1.9 调查结论：the initial `/workflow` gate used
//! `closeclaw_permission::is_owner` (the literal id `"owner"`). That
//! criterion is unreachable in production — the inbound chain passes the
//! raw platform sender through unchanged (SessionRouter
//! `processor_chain/src/session_router.rs` metadata `sender_id`; the
//! terminal channel uses the numeric OS uid,
//! `platform/src/terminal.rs::current_uid`), and no identity mapping
//! rewrites any sender to the literal `"owner"`. The real Owner would
//! therefore have been false-rejected (entry ① unusable), while entries
//! ②③ — comparing the same sender against the checkpoint — recognized
//! them. The gate now uses the checkpoint criterion, and the formerly
//! literal judgments elsewhere (`approval.rs` /approve-*,
//! `slash_permission.rs` Owner short-circuit, permission engine owner
//! short-circuit) have since been migrated to the same checkpoint
//! criterion (issue #3776); the remaining out-of-scope literal-judgment
//! sites are tracked in issue #3791.

use super::{Gateway, HandleResult};

#[cfg(test)]
mod tests;

/// Shared Owner-gate predicate: the inbound `sender` is the session
/// Owner iff it equals the session-bound checkpoint sender id
/// (`SessionManager::get_sender_id`). `None` on either side → not the
/// Owner: sessions without a bound owner grant nothing to anybody, and
/// senderless messages are never the Owner.
///
/// All gateway Owner gates decide through this single predicate —
/// approval commands (`approval.rs`), the slash Owner short-circuit
/// (`slash_permission.rs::SlashRouteCtx::is_owner`), and the workflow
/// entries (below, plus entry ③ in
/// `slash_executor_helpers.rs::terminate_workflow_after_stop`) — so the
/// criterion keeps one definition.
pub(crate) fn sender_is_checkpoint_owner(sender: Option<&str>, owner: Option<&str>) -> bool {
    sender.is_some_and(|sender| owner == Some(sender))
}

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
        let owner_id = self.session_manager.get_sender_id(session_id).await;
        if !sender_is_checkpoint_owner(sender_id, owner_id.as_deref()) {
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
    /// The Owner criterion matches entries ②③: the sender must equal the
    /// session checkpoint's sender id (`SessionManager::get_sender_id`);
    /// sessions without a bound owner grant `/workflow` to nobody.
    ///
    /// Returns `Some(HandleResult::SlashHandled)` after replying
    /// "权限不足" — the command never reaches the SlashDispatcher.
    /// Returns `None` for owner senders and non-`/workflow` content so
    /// routing continues.
    pub(crate) async fn try_intercept_workflow_command(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
        peer_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        let (cmd, _) = super::slash_permission::parse_slash(content)?;
        if cmd != "workflow" {
            return None;
        }
        let owner_id = self.session_manager.get_sender_id(session_id).await;
        if sender_is_checkpoint_owner(sender_id, owner_id.as_deref()) {
            return None;
        }
        tracing::warn!(
            session_id = %session_id,
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
