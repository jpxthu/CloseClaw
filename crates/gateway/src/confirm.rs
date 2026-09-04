//! Plan execution confirmation command handling for the Gateway.
//!
//! Provides `try_handle_plan_confirm_command()` for intercepting
//! `/confirm <id>` and `/cancel <id>` commands from the owner.
//!
//! The gateway stores the handler as `Arc<dyn PlanConfirmationHandler>`
//! (from `closeclaw_common`). The notify callback is installed by the
//! daemon (which has access to both the concrete `PlanExecConfirmFlow`
//! and the Gateway's plugins), avoiding a circular dependency between
//! the gateway and tools crates.
//!
//! This is independent of the permission approval flow — confirmation
//! cards use 📋 prefix and are explicitly unrelated to dangerous-operation
//! approvals (⚠️ prefix).

use super::{Gateway, HandleResult};
use closeclaw_common::plan_confirm_handler::PlanConfirmationHandler;

pub(crate) type PlanConfirmHandler =
    std::sync::Arc<dyn closeclaw_common::plan_confirm_handler::PlanConfirmationHandler>;

/// Invalid UUID format response sent when confirmation_id is not valid.
const INVALID_UUID_MSG: &str = "无效的 confirmation ID 格式。请检查后重试（需为合法 UUID）。";

impl Gateway {
    /// Set the plan execution confirmation handler.
    pub async fn set_plan_confirm_handler(
        &self,
        handler: std::sync::Arc<dyn PlanConfirmationHandler>,
    ) {
        *self.plan_confirm_handler.write().await = Some(handler);
    }

    /// Try to intercept a plan execution confirmation command.
    ///
    /// Supported prefixes (checked in order):
    /// - `/confirm <id>` — confirm a pending plan execution
    /// - `/cancel <id>` — cancel a pending plan execution
    ///
    /// Returns `Some(HandleResult::ApprovalProcessed)` if the command was
    /// handled, or `None` if the message is not a confirm/cancel command.
    ///
    /// Non-owner senders receive a rejection message and the command is
    /// still consumed (prevents fall-through to SlashDispatcher).
    pub(crate) async fn try_handle_plan_confirm_command(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
        peer_id: &str,
        channel: &str,
    ) -> Option<HandleResult> {
        let trimmed = content.trim();

        // Check for /confirm or /cancel prefix.
        let (is_confirm, rest) = if let Some(r) = trimmed.strip_prefix("/confirm") {
            (true, r.trim())
        } else {
            let r = trimmed.strip_prefix("/cancel")?;
            (false, r.trim())
        };

        // Verify sender is the owner
        match sender_id {
            Some("owner") => {}
            _ => {
                // Non-owner: send rejection and consume the command
                tracing::warn!(
                    session_id,
                    sender_id = ?sender_id,
                    "non-owner attempted plan confirm command"
                );
                if let Err(e) = self
                    .send_outbound_simplified(peer_id, channel, "权限不足：该指令仅限 Owner 使用")
                    .await
                {
                    tracing::warn!(
                        session_id,
                        sender_id = ?sender_id,
                        error = %e,
                        "failed to send non-owner rejection message"
                    );
                }
                return Some(HandleResult::ApprovalProcessed);
            }
        }

        // Parse confirmation_id from the rest
        let confirmation_id = rest.split_whitespace().next().unwrap_or("");
        if confirmation_id.is_empty() {
            tracing::warn!(
                session_id,
                "plan confirm command missing confirmation_id: {}",
                trimmed
            );
            return None;
        }

        // Validate UUID format
        if confirmation_id.parse::<uuid::Uuid>().is_err() {
            tracing::warn!(
                session_id,
                confirmation_id,
                "invalid UUID format in plan confirm command"
            );
            if let Err(e) = self
                .send_outbound_simplified(peer_id, channel, INVALID_UUID_MSG)
                .await
            {
                tracing::warn!(session_id, error = %e, "failed to send invalid UUID message");
            }
            return Some(HandleResult::ApprovalProcessed);
        }

        // Get the plan confirmation handler
        let handler_guard = self.plan_confirm_handler.read().await;
        let Some(handler) = handler_guard.as_ref() else {
            tracing::debug!(
                session_id,
                "plan confirm command received but no plan_confirm_handler configured"
            );
            return None;
        };

        // Route to PlanConfirmationHandler
        if is_confirm {
            let ok = handler.confirm(confirmation_id).await;
            if ok {
                tracing::info!(session_id, confirmation_id, "plan execution confirmed");
                if let Err(e) = self
                    .send_outbound_simplified(
                        peer_id,
                        channel,
                        "✅ 已确认执行 plan，进入 Auto Mode。",
                    )
                    .await
                {
                    tracing::warn!(session_id, error = %e, "failed to send confirm feedback");
                }
            } else {
                tracing::warn!(
                    session_id,
                    confirmation_id,
                    "plan confirm request not found or already resolved"
                );
                if let Err(e) = self
                    .send_outbound_simplified(
                        peer_id,
                        channel,
                        "❌ confirmation ID 不存在或已处理，请检查后重试。",
                    )
                    .await
                {
                    tracing::warn!(
                        session_id,
                        error = %e,
                        "failed to send confirm failure feedback"
                    );
                }
            }
        } else {
            let ok = handler.cancel(confirmation_id).await;
            if ok {
                tracing::info!(session_id, confirmation_id, "plan execution cancelled");
                if let Err(e) = self
                    .send_outbound_simplified(peer_id, channel, "✅ 已取消执行 plan。")
                    .await
                {
                    tracing::warn!(session_id, error = %e, "failed to send cancel feedback");
                }
            } else {
                tracing::warn!(session_id, confirmation_id, "plan cancel request not found");
                if let Err(e) = self
                    .send_outbound_simplified(
                        peer_id,
                        channel,
                        "❌ confirmation ID 不存在或已处理，请检查后重试。",
                    )
                    .await
                {
                    tracing::warn!(
                        session_id,
                        error = %e,
                        "failed to send cancel failure feedback"
                    );
                }
            }
        }

        Some(HandleResult::ApprovalProcessed)
    }
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod confirm_tests;
