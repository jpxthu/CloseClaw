//! Gateway-level permission command handlers.
//!
//! Contains the [`Gateway`] methods for new-user registration logic.
//! `/perm` and `/user` commands are now routed through [`SlashDispatcher`]
//! like any other slash command, consistent with the design doc.

use crate::{Gateway, HandleResult};
use closeclaw_permission::UserRegistry;

impl Gateway {
    /// Check if a sender is a new unregistered user and auto-submit
    /// a user creation request via the ApprovalFlow.
    ///
    /// When a non-owner, unregistered user sends their first message:
    /// 1. Submit a user creation request via `ApprovalFlow::submit_user_creation()`
    /// 2. Notify the user that their request is pending approval
    /// 3. Return `Some(HandleResult::SlashHandled)` to block further processing
    ///
    /// Returns `None` if the sender is owner, already registered, or no
    /// approval flow is configured.
    pub(crate) async fn check_new_user_registration(
        &self,
        sender_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        // Owner doesn't need registration.
        if sender_id == "owner" {
            return None;
        }

        // Load user registry from config_dir/users.json.
        let config_dir = self.get_config_dir().await?;
        let registry_path = config_dir.join("users.json");
        let registry: UserRegistry = tokio::fs::read_to_string(&registry_path)
            .await
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default();

        // Already registered → proceed normally.
        if registry.is_registered(sender_id) {
            return None;
        }

        // New user → submit creation request.
        self.submit_new_user_creation(sender_id, channel).await
    }

    /// Submit a user creation request and notify the sender of the result.
    async fn submit_new_user_creation(
        &self,
        sender_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            tracing::debug!(
                sender_id,
                "no approval flow configured, cannot register new user"
            );
            return None;
        };
        let mut flow = flow_arc.lock().await;
        match flow.submit_user_creation(sender_id, channel, vec![]) {
            Some(request_id) => {
                tracing::info!(
                    sender_id,
                    channel,
                    request_id = %request_id,
                    "new user registration request auto-submitted"
                );
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply(format!(
                        "👋 您是新用户，已向 Owner 提交注册申请（请求 ID: {}）。请等待审批。",
                        request_id
                    ))
                    .await;
                }
                Some(HandleResult::SlashHandled)
            }
            None => {
                // Duplicate request or other issue.
                tracing::debug!(
                    sender_id,
                    channel,
                    "user creation request already pending or failed"
                );
                if let Some(sh) = self.session_handler.as_ref() {
                    sh.send_reply("⏳ 您的注册请求正在审批中，请等待。".to_owned())
                        .await;
                }
                Some(HandleResult::SlashHandled)
            }
        }
    }
}
