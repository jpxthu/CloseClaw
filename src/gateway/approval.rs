//! Approval command handling for the Gateway.
//!
//! Provides `set_approval_flow()` for installing the approval flow and
//! `try_handle_approval_command()` for intercepting `/approve` / `/deny`
//! commands from the owner.

use std::collections::HashMap;
use std::sync::Arc;

use crate::permission::approval::ApprovalMode;
use crate::permission::approval_flow::{ApprovalFlow, ApprovalNotification};

use super::{Gateway, HandleResult, Message};

impl Gateway {
    /// Set the approval flow for intercepting `/approve` / `/deny` commands.
    ///
    /// Also installs the `on_notify_owner` callback that sends approval
    /// notifications to the owner via the Gateway's adapters.
    pub async fn set_approval_flow(&self, flow: Arc<tokio::sync::Mutex<ApprovalFlow>>) {
        let handle = tokio::runtime::Handle::current();
        let adapters = self.adapters.read().await;
        // Clone Arc adapter refs for each channel so the callback can send messages.
        let adapter_clones: HashMap<String, Arc<dyn crate::im::IMAdapter>> = adapters
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();
        drop(adapters);

        // Build the on_notify_owner callback.
        // This closure captures the adapter clones and runtime handle to send
        // approval notifications asynchronously.
        let notify_cb = move |notification: ApprovalNotification| {
            let request_id = notification.request_id;
            let agent = notification.caller.agent.clone();
            let user = notification.caller.user_id.clone();
            let op = notification.operation_desc;
            let risk = format!("{:?}", notification.risk_level);

            let text = format!(
                "⚠️ 审批 [{}] Agent [{}] 以 [{}] 执行 [{}] (风险:{})。回复 /approve {} 放行 或 /deny {} 拒绝。",
                request_id, agent, user, op, risk, request_id, request_id
            );

            // Find the first available adapter and send to the owner.
            // The owner channel is determined by the adapter's default target.
            let adapters = adapter_clones.clone();
            let handle = handle.clone();
            handle.spawn(async move {
                // Try each registered adapter until one succeeds.
                for (channel_name, adapter) in &adapters {
                    let msg = Message {
                        id: format!("approval-notify-{}", chrono::Utc::now().timestamp_millis()),
                        from: "system".to_string(),
                        to: "owner".to_string(),
                        content: text.clone(),
                        channel: channel_name.clone(),
                        timestamp: chrono::Utc::now().timestamp(),
                        metadata: std::collections::HashMap::new(),
                    };
                    match adapter.send_message(&msg).await {
                        Ok(()) => break,
                        Err(e) => {
                            tracing::warn!(
                                channel = %channel_name,
                                error = %e,
                                "failed to send approval notification"
                            );
                        }
                    }
                }
            });
        };

        // Set the callback on the ApprovalFlow.
        {
            let mut flow_guard = flow.lock().await;
            flow_guard.set_notify_callback(Arc::new(notify_cb));
        }

        *self.approval_flow.write().await = Some(flow);
    }

    /// Try to intercept an `/approve` or `/deny` approval command.
    ///
    /// Returns `Some(HandleResult::ApprovalProcessed)` if the command was
    /// handled, or `None` if the message is not an approval command (or
    /// the sender is not the owner).
    pub(super) async fn try_handle_approval_command(
        &self,
        session_id: &str,
        content: &str,
        sender_id: Option<&str>,
    ) -> Option<HandleResult> {
        let trimmed = content.trim();

        // Check for /approve or /deny prefix
        let (is_approve, rest) = if let Some(r) = trimmed.strip_prefix("/approve") {
            (true, r.trim())
        } else if let Some(r) = trimmed.strip_prefix("/deny") {
            (false, r.trim())
        } else {
            return None; // Not an approval command
        };

        // Verify sender is the owner
        match sender_id {
            Some(id) if id == "owner" => {}
            _ => return None, // Not owner — fall through to normal message flow
        }

        // Parse request_id from the rest
        let request_id = rest.split_whitespace().next().unwrap_or("");
        if request_id.is_empty() {
            tracing::warn!(
                session_id,
                "approval command missing request_id: {}",
                trimmed
            );
            return None;
        }

        // Determine approval mode (--whitelist flag)
        let mode = if rest.contains("--whitelist") {
            ApprovalMode::WithWhitelist
        } else {
            ApprovalMode::Once
        };

        // Get the approval flow
        let flow_guard = self.approval_flow.read().await;
        let Some(flow_arc) = flow_guard.as_ref() else {
            tracing::debug!(
                session_id,
                "approval command received but no approval_flow configured"
            );
            return None;
        };

        // Route to ApprovalFlow
        let mut flow = flow_arc.lock().await;
        if is_approve {
            match flow.approve_request(request_id, mode) {
                Ok(true) => {
                    tracing::info!(session_id, request_id, ?mode, "approval request approved");
                    return Some(HandleResult::ApprovalProcessed);
                }
                Ok(false) => {
                    tracing::warn!(
                        session_id,
                        request_id,
                        "approval request not found or already resolved"
                    );
                    return Some(HandleResult::ApprovalProcessed);
                }
                Err(e) => {
                    tracing::warn!(
                        session_id,
                        request_id,
                        error = ?e,
                        "approval request rejected"
                    );
                    // Still consumed the command — don't fall through to LLM
                    return Some(HandleResult::ApprovalProcessed);
                }
            }
        } else {
            let denied = flow.deny_request(request_id);
            tracing::info!(session_id, request_id, denied, "approval request denied");
            return Some(HandleResult::ApprovalProcessed);
        }
    }
}
