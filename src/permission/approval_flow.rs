//! ApprovalFlow - Daemon-level approval orchestrator
//!
//! Wraps [`ApprovalQueue`] and integrates with [`SessionManager`] to provide
//! the full approval workflow: deny → queue → notify owner → approve/deny →
//! push result message to session.
//!
//! # Architecture
//!
//! ```text
//! Tool call → Deny → submit_denial()
//!                     ├─ sub_agent? → None (silent deny)
//!                     ├─ heartbeat? → None (silent skip)
//!                     └─ normal?    → enqueue → on_notify_owner → Some(id)
//!
//! Owner → /approve id → approve_request(id, Once)
//!         └─ lookup session_id → queue.approve() → spawn push "已批准" to session
//!
//! Owner → /deny id → deny_request(id)
//!         └─ lookup session_id → queue.deny() → spawn push "已拒绝" to session
//! ```

use std::sync::Arc;

use crate::gateway::session_manager::SessionManager;
use crate::permission::engine::engine_risk::RiskLevel;
use crate::permission::engine::engine_types::{Caller, PermissionRequestBody};
use crate::session::persistence::PendingMessage;

use super::approval::{ApprovalMode, ApprovalQueue, ApproveOrDeny, RejectWhitelistReason};

/// Notification sent to the owner when an operation requires approval.
#[derive(Debug, Clone)]
pub struct ApprovalNotification {
    /// Unique request identifier.
    pub request_id: String,
    /// Caller that initiated the operation.
    pub caller: Caller,
    /// Human-readable description of the operation.
    pub operation_desc: String,
    /// Risk level of the operation.
    pub risk_level: RiskLevel,
}

/// Operations that should be silently skipped during the approval flow.
///
/// In the initial implementation this is a hardcoded set; later this may
/// become configurable via permissions.json.
fn is_heartbeat_skip_operation(request: &PermissionRequestBody) -> bool {
    matches!(
        request,
        PermissionRequestBody::ToolCall {
            skill,
            method,
            ..
        } if skill == "heartbeat" && method == "ping"
    )
}

/// Daemon-level approval orchestrator.
///
/// Holds the [`ApprovalQueue`], a reference to [`SessionManager`] for pushing
/// result messages, an owner notification callback, and a tokio runtime handle
/// for spawning async tasks from synchronous closures.
pub struct ApprovalFlow {
    /// The underlying approval queue.
    queue: ApprovalQueue,
    /// Session manager for pushing pending messages.
    session_manager: Arc<SessionManager>,
    /// Callback invoked to notify the owner about a pending approval.
    on_notify_owner: Arc<dyn Fn(ApprovalNotification) + Send + Sync>,
    /// Tokio runtime handle for spawning async tasks from sync closures.
    runtime_handle: tokio::runtime::Handle,
}

impl std::fmt::Debug for ApprovalFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalFlow")
            .field("queue", &self.queue)
            .finish_non_exhaustive()
    }
}

impl ApprovalFlow {
    /// Create a new `ApprovalFlow`.
    ///
    /// # Arguments
    /// * `session_manager` - Shared reference to the session manager.
    /// * `on_notify_owner` - Callback to notify the owner about pending approvals.
    /// * `runtime_handle` - Tokio runtime handle for spawning async tasks.
    pub fn new(
        session_manager: Arc<SessionManager>,
        on_notify_owner: Arc<dyn Fn(ApprovalNotification) + Send + Sync>,
        runtime_handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            queue: ApprovalQueue::new(),
            session_manager,
            on_notify_owner,
            runtime_handle,
        }
    }

    /// Replace the owner notification callback.
    ///
    /// Used by the Gateway to inject a callback that sends notifications
    /// through the registered IM adapters.
    pub fn set_notify_callback(&mut self, cb: Arc<dyn Fn(ApprovalNotification) + Send + Sync>) {
        self.on_notify_owner = cb;
    }

    /// Submit a denied operation for owner approval.
    ///
    /// # Behavior
    /// - `is_sub_agent = true` → returns `None` (silent deny, no queue).
    /// - Heartbeat-skip operations → returns `None` (silent skip).
    /// - Normal operations → enqueues (dedup via `ApprovalQueue`) → triggers
    ///   `on_notify_owner` → returns `Some(request_id)`.
    ///
    /// # Deduplication
    /// If an equivalent request (same caller + same operation body) is already
    /// pending in the queue, `ApprovalQueue::enqueue` rejects it as a duplicate
    /// and this method returns `None`.
    pub fn submit_denial(
        &mut self,
        caller: &Caller,
        request: &PermissionRequestBody,
        risk_level: RiskLevel,
        session_id: &str,
        is_sub_agent: bool,
    ) -> Option<String> {
        // Sub-agent operations are silently denied.
        if is_sub_agent {
            return None;
        }

        // Heartbeat-skip operations are silently skipped.
        if is_heartbeat_skip_operation(request) {
            return None;
        }

        let operation_desc = match request {
            PermissionRequestBody::FileOp { agent, path, op } => {
                format!("{} file {} {}", agent, op, path)
            }
            PermissionRequestBody::CommandExec { agent, cmd, .. } => {
                format!("{} execute {}", agent, cmd)
            }
            PermissionRequestBody::NetOp { agent, host, port } => {
                format!("{} network {}:{}", agent, host, port)
            }
            PermissionRequestBody::ToolCall {
                agent,
                skill,
                method,
            } => {
                format!("{} tool {}/{}", agent, skill, method)
            }
            PermissionRequestBody::InterAgentMsg { from, to } => {
                format!("inter-agent {} -> {}", from, to)
            }
            PermissionRequestBody::ConfigWrite { agent, config_file } => {
                format!("{} config write {}", agent, config_file)
            }
        };

        let session_resume = session_id.to_string();
        let risk = risk_level;

        // Callback: no-op. The actual push_pending_message calls are in
        // approve_request / deny_request to avoid duplicate messages.
        let callback = Box::new(move |_result: ApproveOrDeny| {});

        let request_id = match self.queue.enqueue(
            request.clone(),
            caller.clone(),
            operation_desc.clone(),
            risk,
            session_resume,
            callback,
        ) {
            Ok(id) => id,
            Err(_) => return None, // Duplicate — silently ignore.
        };

        // Notify the owner about the pending approval.
        (self.on_notify_owner)(ApprovalNotification {
            request_id: request_id.clone(),
            caller: caller.clone(),
            operation_desc,
            risk_level: risk,
        });

        Some(request_id)
    }

    /// Approve a pending approval request.
    ///
    /// Delegates to [`ApprovalQueue::approve`] with the given [`ApprovalMode`].
    /// On success, a "已批准" message is pushed to the requesting session.
    ///
    /// # Errors
    /// Returns `Err(RejectWhitelistReason::HighRisk)` if `mode` is
    /// `WithWhitelist` and the operation's risk level is High or Critical.
    pub fn approve_request(
        &mut self,
        request_id: &str,
        mode: ApprovalMode,
    ) -> Result<bool, RejectWhitelistReason> {
        // Extract session_id BEFORE resolving (entry is removed on resolve).
        let session_resume = self
            .queue
            .get_pending(request_id)
            .map(|p| p.session_resume.clone());

        let result = self.queue.approve(request_id, mode)?;

        if result {
            if let Some(session_id) = session_resume {
                let sm = Arc::clone(&self.session_manager);
                let handle = self.runtime_handle.clone();
                let rid = request_id.to_string();

                handle.spawn(async move {
                    let content = format!("[审批 {}] 操作已批准", rid);
                    let msg = PendingMessage::new(
                        format!("approval-{}", chrono::Utc::now().timestamp_millis()),
                        content,
                    );
                    if let Err(e) = sm.push_pending_message(&session_id, msg).await {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "failed to push approval result to session"
                        );
                    }
                });
            }
        }

        Ok(result)
    }

    /// Deny a pending approval request.
    ///
    /// Delegates to [`ApprovalQueue::deny`]. On success, a "已拒绝" message
    /// is pushed to the requesting session.
    pub fn deny_request(&mut self, request_id: &str) -> bool {
        // Extract session_id BEFORE resolving (entry is removed on resolve).
        let session_resume = self
            .queue
            .get_pending(request_id)
            .map(|p| p.session_resume.clone());

        let result = self.queue.deny(request_id);

        if result {
            if let Some(session_id) = session_resume {
                let sm = Arc::clone(&self.session_manager);
                let handle = self.runtime_handle.clone();
                let rid = request_id.to_string();

                handle.spawn(async move {
                    let content = format!("[审批 {}] 操作已拒绝", rid);
                    let msg = PendingMessage::new(
                        format!("approval-{}", chrono::Utc::now().timestamp_millis()),
                        content,
                    );
                    if let Err(e) = sm.push_pending_message(&session_id, msg).await {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "failed to push denial result to session"
                        );
                    }
                });
            }
        }

        result
    }

    /// Clear all pending approvals.
    ///
    /// All pending requests are denied with callbacks triggered.
    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests;
