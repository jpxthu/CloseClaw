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
//!                     ├─ heartbeat? → mode-dependent:
//!                     │     Skip  → None (silent)
//!                     │     Notify → notify owner, None
//!                     │     Ask   → enqueue (same as normal)
//!                     └─ normal?    → enqueue → on_notify_owner → Some(id)
//!
//! Owner → /approve id → approve_request(id, Once)
//!         └─ lookup session_id → queue.approve() → spawn push "已批准" to session
//!
//! Owner → /deny id → deny_request(id)
//!         └─ lookup session_id → queue.deny() → spawn push "已拒绝" to session
//! ```

use std::sync::Arc;

use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequestBody};
use closeclaw_common::{PendingMessage, SessionLookup};

use super::approval::{ApprovalMode, ApprovalQueue, ApproveOrDeny, RejectWhitelistReason};

/// How heartbeat operations are handled when denied by the permission engine.
///
/// This controls the approval flow behavior for heartbeat tasks that receive
/// a Deny verdict from the permission engine:
///
/// - [`Skip`](HeartbeatApprovalMode::Skip): Silently skip the operation (default).
///   Heartbeat denials are not enqueued and no notification is sent.
/// - [`Notify`](HeartbeatApprovalMode::Notify): Notify the owner about the
///   denial but do not enqueue for approval. This is a one-way notification.
/// - [`Ask`](HeartbeatApprovalMode::Ask): Enqueue the heartbeat denial for
///   owner approval, treating it the same as any other denied operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HeartbeatApprovalMode {
    /// Silently skip denied heartbeat operations (no queue, no notification).
    #[default]
    Skip,
    /// Notify the owner about the denial but do not enqueue for approval.
    Notify,
    /// Enqueue denied heartbeat operations for owner approval.
    Ask,
}

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

/// Check if a request is a heartbeat operation.
///
/// Heartbeat operations are tool calls with skill="heartbeat" and
/// method="ping". The handling strategy (skip / notify / ask) is
/// determined by [`HeartbeatApprovalMode`].
fn is_heartbeat_operation(request: &PermissionRequestBody) -> bool {
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
    session_manager: Arc<dyn SessionLookup>,
    /// Callback invoked to notify the owner about a pending approval.
    on_notify_owner: Arc<dyn Fn(ApprovalNotification) + Send + Sync>,
    /// Tokio runtime handle for spawning async tasks from sync closures.
    runtime_handle: tokio::runtime::Handle,
    /// How heartbeat operations are handled when denied.
    heartbeat_mode: HeartbeatApprovalMode,
}

impl std::fmt::Debug for ApprovalFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalFlow")
            .field("queue", &self.queue)
            .field("heartbeat_mode", &self.heartbeat_mode)
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
    /// * `heartbeat_mode` - How heartbeat operations are handled when denied.
    pub fn new(
        session_manager: Arc<dyn SessionLookup>,
        on_notify_owner: Arc<dyn Fn(ApprovalNotification) + Send + Sync>,
        runtime_handle: tokio::runtime::Handle,
        heartbeat_mode: HeartbeatApprovalMode,
    ) -> Self {
        Self {
            queue: ApprovalQueue::new(),
            session_manager,
            on_notify_owner,
            runtime_handle,
            heartbeat_mode,
        }
    }

    /// Replace the owner notification callback.
    ///
    /// Used by the Gateway to inject a callback that sends notifications
    /// through the registered IM adapters.
    pub fn set_notify_callback(&mut self, cb: Arc<dyn Fn(ApprovalNotification) + Send + Sync>) {
        self.on_notify_owner = cb;
    }

    /// Set the heartbeat approval mode at runtime.
    ///
    /// Allows changing how heartbeat denials are handled without
    /// recreating the [`ApprovalFlow`].
    pub fn set_heartbeat_mode(&mut self, mode: HeartbeatApprovalMode) {
        self.heartbeat_mode = mode;
    }

    /// Handle a denied heartbeat operation according to the configured mode.
    ///
    /// Returns `None` if the operation should not be enqueued (Skip/Notify modes),
    /// or `Some(())` if it should proceed to the normal enqueue flow (Ask mode).
    fn handle_heartbeat_denial(
        &self,
        caller: &Caller,
        request: &PermissionRequestBody,
        risk_level: RiskLevel,
    ) -> Option<String> {
        match self.heartbeat_mode {
            HeartbeatApprovalMode::Skip => None,
            HeartbeatApprovalMode::Notify => {
                if let PermissionRequestBody::ToolCall {
                    agent,
                    skill,
                    method,
                } = request
                {
                    (self.on_notify_owner)(ApprovalNotification {
                        request_id: String::new(),
                        caller: caller.clone(),
                        operation_desc: format!("{} tool {}/{}", agent, skill, method),
                        risk_level,
                    });
                }
                None
            }
            HeartbeatApprovalMode::Ask => Some(String::new()),
        }
    }

    /// Build a human-readable description of the operation for notifications.
    fn format_operation_desc(request: &PermissionRequestBody) -> String {
        match request {
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
            } => format!("{} tool {}/{}", agent, skill, method),
            PermissionRequestBody::InterAgentMsg { from, to } => {
                format!("inter-agent {} -> {}", from, to)
            }
            PermissionRequestBody::ConfigWrite { agent, config_file } => {
                format!("{} config write {}", agent, config_file)
            }
            PermissionRequestBody::SlashCommand { agent, command } => {
                format!("{} slash /{}", agent, command)
            }
        }
    }

    /// Submit a denied operation for owner approval.
    ///
    /// # Behavior
    /// - `is_sub_agent = true` → returns `None` (silent deny, no queue).
    /// - Heartbeat operations → handled according to [`HeartbeatApprovalMode`]:
    ///   - `Skip` → returns `None` (silent skip, no notification).
    ///   - `Notify` → sends owner notification, returns `None` (no queue).
    ///   - `Ask` → enqueues for approval like normal operations.
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
        if is_sub_agent {
            return None;
        }
        if is_heartbeat_operation(request)
            && self
                .handle_heartbeat_denial(caller, request, risk_level)
                .is_none()
        {
            return None;
        }
        let operation_desc = Self::format_operation_desc(request);
        let callback = Box::new(|_: ApproveOrDeny| {});
        let request_id = self
            .queue
            .enqueue(
                request.clone(),
                caller.clone(),
                operation_desc.clone(),
                risk_level,
                session_id.to_string(),
                callback,
            )
            .ok()?;
        (self.on_notify_owner)(ApprovalNotification {
            request_id: request_id.clone(),
            caller: caller.clone(),
            operation_desc,
            risk_level,
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
