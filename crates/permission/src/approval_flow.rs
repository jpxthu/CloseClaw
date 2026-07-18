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

#[path = "approval_flow_user_creation.rs"]
mod approval_flow_user_creation;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::engine::engine_eval::PermissionEngine;
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{
    Caller, PermissionRequest, PermissionRequestBody, PermissionResponse, RuleSet,
};
use closeclaw_common::permission_op::{InitialPermissionSet, UserCreationRequest};
use closeclaw_common::{
    ModeTransition, PendingMessage, PlanPhase, PlanStatus, SessionLookup, SessionMode,
};

use closeclaw_session::plan_file;

use super::approval::{
    ApprovalMode, ApprovalQueue, ApproveOrDeny, EnqueueRequest, RejectWhitelistReason,
};

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

/// Callback type for creating a child session (new-session execution path).
///
/// The daemon layer injects this callback to provide session creation
/// capability without introducing a direct dependency from the permission
/// crate on the gateway crate.
///
/// # Arguments
/// * `parent_session_id` — ID of the session that requested plan execution.
/// * `plan_content` — Full content of the plan file to inject as initial context.
/// * `step_selection` — Optional step indices to execute (passed through to plan state).
///
/// # Returns
/// `Ok(new_session_id)` on success, `Err(message)` on failure.
pub type CreateChildSessionFn = Arc<
    dyn Fn(
            String,
            String,
            Option<Vec<usize>>,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Metadata for a plan execution approval request.
///
/// Stored by [`ApprovalFlow::set_plan_exec_metadata`] and consumed by
/// [`ApprovalFlow::approve_request`] when the owner approves the request.
#[derive(Debug, Clone)]
pub struct PlanExecMetadata {
    /// Path to the plan file to execute.
    pub plan_file_path: String,
    /// Optional step selection (0-based indices of steps to execute).
    pub step_selection: Option<Vec<usize>>,
    /// Whether to create a new child session for execution.
    pub new_session: bool,
}

/// Daemon-level approval orchestrator.
///
/// Holds the [`ApprovalQueue`], a reference to [`SessionManager`] for pushing
/// result messages, an owner notification callback, a whitelist-updated
/// callback, and a tokio runtime handle for spawning async tasks from
/// synchronous closures.
pub struct ApprovalFlow {
    /// The underlying approval queue.
    queue: ApprovalQueue,
    /// Session manager for pushing pending messages.
    session_manager: Arc<dyn SessionLookup>,
    /// Callback invoked to notify the owner about a pending approval.
    on_notify_owner: Arc<dyn Fn(ApprovalNotification) + Send + Sync>,
    /// Callback invoked after a whitelist rule is persisted.
    ///
    /// The parameter is the `agent_id` whose `permissions.json` was updated.
    /// The daemon layer injects the actual permission engine reload logic.
    on_whitelist_updated: Arc<dyn Fn(&str) + Send + Sync>,
    /// Tokio runtime handle for spawning async tasks from sync closures.
    runtime_handle: tokio::runtime::Handle,
    /// How heartbeat operations are handled when denied.
    heartbeat_mode: HeartbeatApprovalMode,
    /// Root config directory for agent permissions persistence.
    config_dir: PathBuf,
    /// Pending user creation requests keyed by request_id.
    user_creation_requests: HashMap<String, UserCreationRequest>,
    /// Current effective rule set (snapshot for approval flow).
    current_rules: RuleSet,
    /// When true, `submit_denial` always returns `None` (silent deny).
    ///
    /// Defaults to `false` in production. Tests set this to `true` to
    /// simulate a hard-denial path where the approval flow does not
    /// accept the request for owner approval.
    force_deny: bool,
    /// Plan execution metadata keyed by request_id.
    ///
    /// Stores metadata for plan execution approval requests (`new_session`,
    /// `step_selection`). Consumed by `approve_request` when the approval
    /// decision is made.
    plan_exec_metadata: HashMap<String, PlanExecMetadata>,
    /// Callback for creating child sessions (new-session execution path).
    ///
    /// Injected by the daemon layer to provide session creation capability
    /// without a direct dependency on `SessionManager`. When `None`,
    /// the new-session execution path falls back to same-session behavior.
    create_child_session_fn: Option<CreateChildSessionFn>,
}

impl std::fmt::Debug for ApprovalFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalFlow")
            .field("queue", &self.queue)
            .field("heartbeat_mode", &self.heartbeat_mode)
            .field("pending_user_creations", &self.user_creation_requests.len())
            .field("force_deny", &self.force_deny)
            .finish_non_exhaustive()
    }
}

impl ApprovalFlow {
    /// Create a new `ApprovalFlow`.
    ///
    /// # Arguments
    /// * `session_manager` - Shared reference to the session manager.
    /// * `on_notify_owner` - Callback to notify the owner about pending approvals.
    /// * `on_whitelist_updated` - Callback invoked after a whitelist rule is
    ///   persisted (parameter: agent_id). The daemon layer injects the actual
    ///   permission engine reload logic here.
    /// * `runtime_handle` - Tokio runtime handle for spawning async tasks.
    /// * `heartbeat_mode` - How heartbeat operations are handled when denied.
    /// * `config_dir` - Root config directory for agent permissions persistence.
    pub fn new(
        session_manager: Arc<dyn SessionLookup>,
        on_notify_owner: Arc<dyn Fn(ApprovalNotification) + Send + Sync>,
        on_whitelist_updated: Arc<dyn Fn(&str) + Send + Sync>,
        runtime_handle: tokio::runtime::Handle,
        heartbeat_mode: HeartbeatApprovalMode,
        config_dir: PathBuf,
        initial_rules: RuleSet,
    ) -> Self {
        Self {
            queue: ApprovalQueue::new(),
            session_manager,
            on_notify_owner,
            on_whitelist_updated,
            runtime_handle,
            heartbeat_mode,
            config_dir,
            user_creation_requests: HashMap::new(),
            current_rules: initial_rules,
            force_deny: false,
            plan_exec_metadata: HashMap::new(),
            create_child_session_fn: None,
        }
    }

    /// Create an `ApprovalFlow` that always denies (for tests).
    ///
    /// `submit_denial` returns `None` unconditionally, simulating a
    /// hard-denial path where the approval flow does not accept the
    /// request for owner approval.
    pub fn new_deny_all(
        session_manager: Arc<dyn SessionLookup>,
        on_notify_owner: Arc<dyn Fn(ApprovalNotification) + Send + Sync>,
        on_whitelist_updated: Arc<dyn Fn(&str) + Send + Sync>,
        runtime_handle: tokio::runtime::Handle,
        heartbeat_mode: HeartbeatApprovalMode,
        config_dir: PathBuf,
        initial_rules: RuleSet,
    ) -> Self {
        Self {
            queue: ApprovalQueue::new(),
            session_manager,
            on_notify_owner,
            on_whitelist_updated,
            runtime_handle,
            heartbeat_mode,
            config_dir,
            user_creation_requests: HashMap::new(),
            current_rules: initial_rules,
            force_deny: true,
            plan_exec_metadata: HashMap::new(),
            create_child_session_fn: None,
        }
    }

    /// Replace the owner notification callback.
    ///
    /// Used by the Gateway to inject a callback that sends notifications
    /// through the registered IM adapters.
    pub fn set_notify_callback(&mut self, cb: Arc<dyn Fn(ApprovalNotification) + Send + Sync>) {
        self.on_notify_owner = cb;
    }

    /// Replace the whitelist-updated callback.
    ///
    /// Used by the Daemon to inject the permission engine reload logic.
    pub fn set_whitelist_callback(&mut self, cb: Arc<dyn Fn(&str) + Send + Sync>) {
        self.on_whitelist_updated = cb;
    }

    /// Set the callback for creating child sessions.
    ///
    /// Called by the daemon layer to inject session creation capability
    /// for the new-session execution path.
    pub fn set_create_child_session_fn(&mut self, cb: CreateChildSessionFn) {
        self.create_child_session_fn = Some(cb);
    }

    /// Store plan execution metadata for a pending approval request.
    ///
    /// Called by [`ExecutePlanTool`](crate) before returning
    /// `approval_pending`. The metadata is consumed by [`approve_request`]
    /// when the owner approves the request.
    pub fn set_plan_exec_metadata(
        &mut self,
        request_id: &str,
        plan_file_path: String,
        step_selection: Option<Vec<usize>>,
        new_session: bool,
    ) {
        self.plan_exec_metadata.insert(
            request_id.to_string(),
            PlanExecMetadata {
                plan_file_path,
                step_selection,
                new_session,
            },
        );
    }
}

// ── Heartbeat mode ──────────────────────────────────────────────────────────

impl ApprovalFlow {
    /// Set the heartbeat approval mode at runtime.
    ///
    /// Allows changing how heartbeat denials are handled without
    /// recreating the [`ApprovalFlow`].
    pub fn set_heartbeat_mode(&mut self, mode: HeartbeatApprovalMode) {
        self.heartbeat_mode = mode;
    }

    /// Update the current rule set snapshot.
    ///
    /// Called by the permission engine hot-reload path to keep the
    /// approval flow's rule snapshot in sync with the live rules.
    pub fn update_rules(&mut self, rules: RuleSet) {
        self.current_rules = rules;
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
}

// ── Operation submission ────────────────────────────────────────────────────

impl ApprovalFlow {
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
            PermissionRequestBody::MessageSend {
                agent,
                direction,
                target,
            } => {
                format!("{} message {:?} {}", agent, direction, target)
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
        if self.force_deny {
            return None;
        }
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
                EnqueueRequest {
                    request: request.clone(),
                    caller: caller.clone(),
                    operation_desc: operation_desc.clone(),
                    risk_level,
                    session_resume: session_id.to_string(),
                    callback,
                },
                &self.current_rules,
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
}

// ── User creation submission ──────────────────────────────────────────────

impl ApprovalFlow {
    /// Submit a user creation request for owner approval.
    ///
    /// Stores the request and notifies the owner via [`ApprovalNotification`].
    /// The request is resolved when the owner calls [`approve_request`] or
    /// [`deny_request`] with the returned `request_id`.
    ///
    /// Returns `Some(request_id)` on success, or `None` if a duplicate
    /// request (same user_id + channel) is already pending.
    pub fn submit_user_creation(
        &mut self,
        user_id: &str,
        channel: &str,
        initial_permissions: Vec<InitialPermissionSet>,
    ) -> Option<String> {
        // Dedup: check if same user+channel already pending.
        let is_dup = self
            .user_creation_requests
            .values()
            .any(|r| r.user_id == user_id && r.im_channel == channel);
        if is_dup {
            return None;
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let request = UserCreationRequest {
            user_id: user_id.to_string(),
            im_channel: channel.to_string(),
            request_id: request_id.clone(),
            initial_permissions,
        };
        self.user_creation_requests
            .insert(request_id.clone(), request);

        (self.on_notify_owner)(ApprovalNotification {
            request_id: request_id.clone(),
            caller: Caller {
                user_id: user_id.to_string(),
                agent: String::new(),
                creator_id: String::new(),
            },
            operation_desc: format!("新用户注册请求：{} 通过 {} 渠道", user_id, channel),
            risk_level: RiskLevel::Low,
        });

        Some(request_id)
    }

    /// Update the initial permissions for a pending user creation request.
    ///
    /// Called by `/user approve --perms <set>` to set the permission template
    /// before the request is approved.
    pub fn set_user_creation_permissions(
        &mut self,
        request_id: &str,
        initial_permissions: Vec<InitialPermissionSet>,
    ) -> bool {
        if let Some(req) = self.user_creation_requests.get_mut(request_id) {
            req.initial_permissions = initial_permissions;
            true
        } else {
            false
        }
    }
}

// ── Approval resolution ─────────────────────────────────────────────────────

impl ApprovalFlow {
    /// Approve a pending approval request.
    ///
    /// Delegates to [`ApprovalQueue::approve`] with the given [`ApprovalMode`].
    /// On success, a "已批准" message is pushed to the requesting session.
    ///
    /// For user creation requests, the user is registered via
    /// [`UserRegistry`] and initial permission rules are persisted.
    ///
    /// # Errors
    /// Returns `Err(RejectWhitelistReason::HighRisk)` if `mode` is
    /// `WithWhitelist` and the operation's risk level is High or Critical.
    pub async fn approve_request(
        &mut self,
        request_id: &str,
        mode: ApprovalMode,
    ) -> Result<bool, RejectWhitelistReason> {
        // Check if this is a pending user creation request first.
        if let Some(uc_request) = self.user_creation_requests.remove(request_id) {
            let registered = self.approve_user_creation(&uc_request).await;
            return Ok(registered);
        }

        // Extract details BEFORE resolving (entry is removed on resolve).
        let pending_info = self.queue.get_pending(request_id).map(|p| {
            (
                p.session_resume.clone(),
                p.caller.clone(),
                p.request.clone(),
                p.snapshotted_rules.clone(),
                p.rule_version.clone(),
            )
        });

        let effective_mode =
            self.reevaluate_with_snapshotted_rules(request_id, &pending_info, mode);
        let final_mode = effective_mode.unwrap_or(mode);
        let result = self.queue.approve(request_id, final_mode)?;

        self.persist_whitelist(request_id, &pending_info, final_mode, result);
        self.handle_plan_approval(request_id, &pending_info, result)
            .await;

        Ok(result)
    }

    /// Deny a pending approval request.
    ///
    /// Delegates to [`ApprovalQueue::deny`]. On success, a "已拒绝" message
    /// is pushed to the requesting session.
    pub fn deny_request(&mut self, request_id: &str) -> bool {
        // Check user creation requests first.
        if self.user_creation_requests.remove(request_id).is_some() {
            return true;
        }

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
                    let msg = PendingMessage::with_role(
                        format!("approval-{}", chrono::Utc::now().timestamp_millis()),
                        content,
                        "assistant".to_string(),
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

// ── Approval helpers ───────────────────────────────────────────────────────

type PendingInfo = (
    String,                // session_resume
    Caller,                // caller
    PermissionRequestBody, // request
    RuleSet,               // snapshotted_rules
    String,                // rule_version
);

impl ApprovalFlow {
    /// Re-evaluate with snapshotted rules to check if the snapshot
    /// rules already allow the operation (owner decision not needed).
    fn reevaluate_with_snapshotted_rules(
        &self,
        request_id: &str,
        pending_info: &Option<PendingInfo>,
        _mode: ApprovalMode,
    ) -> Option<ApprovalMode> {
        let (_, ref caller, ref request, ref snapshotted_rules, _) = pending_info.as_ref()?;
        let temp_engine = PermissionEngine::new_with_default_data_root(snapshotted_rules.clone());
        let perm_request = PermissionRequest::WithCaller {
            caller: caller.clone(),
            request: request.clone(),
        };
        let re_result = temp_engine.evaluate(perm_request, None);
        match re_result {
            PermissionResponse::Allowed { .. } => {
                tracing::info!(
                    request_id = %request_id,
                    "规则已变更，操作已自动放行"
                );
                Some(ApprovalMode::Once)
            }
            _ => None,
        }
    }

    /// Persist whitelist rule after approval (best-effort).
    fn persist_whitelist(
        &self,
        request_id: &str,
        pending_info: &Option<PendingInfo>,
        final_mode: ApprovalMode,
        result: bool,
    ) {
        let target = match final_mode {
            ApprovalMode::WithWhitelist { target } => target,
            _ => return,
        };
        if !result {
            return;
        }
        if let Some((_, ref caller, ref request, _, ref rule_version)) = pending_info {
            let name = format!(
                "whitelist-{}-{}",
                chrono::Utc::now().timestamp_millis(),
                rule_version
            );
            if let Some(rule) =
                crate::whitelist::build_whitelist_rule(caller, request, &name, target)
            {
                if let Err(e) =
                    crate::whitelist::append_whitelist_rule(&self.config_dir, &caller.agent, rule)
                {
                    tracing::warn!(
                        request_id = %request_id,
                        agent = %caller.agent,
                        error = %e,
                        "failed to persist whitelist rule (best-effort)"
                    );
                } else {
                    (self.on_whitelist_updated)(&caller.agent);
                }
            }
        }
    }

    /// Handle plan approval: push result and transition session to Auto Mode.
    async fn handle_plan_approval(
        &mut self,
        request_id: &str,
        pending_info: &Option<PendingInfo>,
        result: bool,
    ) {
        if !result {
            return;
        }
        let session_id = match pending_info {
            Some((sid, _, _, _, _)) => sid.clone(),
            None => return,
        };
        let sm = Arc::clone(&self.session_manager);
        let handle = self.runtime_handle.clone();
        let rid = request_id.to_string();
        let plan_meta = self.plan_exec_metadata.remove(&rid);
        let create_child_fn = self.create_child_session_fn.clone();

        handle.spawn(async move {
            Self::push_approval_result(&sm, &session_id, &rid).await;
            Self::transition_plan_to_auto(&sm, &session_id, plan_meta, &create_child_fn).await;
        });
    }

    /// Push the approval result message to the session.
    async fn push_approval_result(sm: &Arc<dyn SessionLookup>, session_id: &str, rid: &str) {
        let content = format!("[审批 {}] 操作已批准", rid);
        let msg = PendingMessage::with_role(
            format!("approval-{}", chrono::Utc::now().timestamp_millis()),
            content,
            "assistant".to_string(),
        );
        if let Err(e) = sm.push_pending_message(session_id, msg).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to push approval result to session"
            );
        }
    }

    /// Transition the plan session to Auto Mode (same-session or new-session).
    async fn transition_plan_to_auto(
        sm: &Arc<dyn SessionLookup>,
        session_id: &str,
        plan_meta: Option<PlanExecMetadata>,
        create_child_session_fn: &Option<CreateChildSessionFn>,
    ) {
        let mut plan_state = match sm.get_plan_state(session_id).await {
            Some(ps) => ps,
            None => return,
        };
        if plan_state.plan_file_path.is_empty() {
            return;
        }
        let is_new_session = plan_meta.as_ref().map(|m| m.new_session).unwrap_or(false);
        if is_new_session {
            Self::handle_new_session_path(
                sm,
                session_id,
                &mut plan_state,
                &plan_meta,
                create_child_session_fn,
            )
            .await;
        } else {
            Self::handle_same_session_path(sm, session_id, &mut plan_state, &plan_meta).await;
        }
    }

    /// Handle new-session execution path: create a child session with plan
    /// content injected as initial context, then enter Auto Mode.
    ///
    /// Falls back to same-session behavior (update plan state only) when
    /// no `create_child_session_fn` callback is configured.
    async fn handle_new_session_path(
        sm: &Arc<dyn SessionLookup>,
        session_id: &str,
        plan_state: &mut closeclaw_common::PlanState,
        plan_meta: &Option<PlanExecMetadata>,
        create_child_session_fn: &Option<CreateChildSessionFn>,
    ) {
        let plan_file_path = plan_state.plan_file_path.clone();

        // Read plan file content for injection into child session.
        let plan_content = match tokio::fs::read_to_string(&plan_file_path).await {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    plan_file = %plan_file_path,
                    error = %e,
                    "failed to read plan file for new session injection"
                );
                return;
            }
        };

        // Create child session via the injected callback.
        let new_session_id = match create_child_session_fn {
            Some(ref create_fn) => {
                let step_selection = plan_meta.as_ref().and_then(|m| m.step_selection.clone());
                match create_fn(session_id.to_string(), plan_content, step_selection).await {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!(
                            parent_session = %session_id,
                            error = %e,
                            "failed to create child session for plan execution"
                        );
                        return;
                    }
                }
            }
            None => {
                tracing::info!(
                    parent_session = %session_id,
                    "no create_child_session_fn configured, falling back to same-session plan state update"
                );
                // Fallback: update plan state on the parent session (same-session behavior).
                plan_state.phase = PlanPhase::FinalPlan;
                plan_state.step_selection =
                    plan_meta.as_ref().and_then(|m| m.step_selection.clone());
                sm.set_plan_state(session_id, plan_state.clone()).await;
                return;
            }
        };

        // Update plan file status to Executing.
        Self::update_plan_file_executing(&plan_file_path).await;

        // Set plan state on the new child session.
        let mut child_plan_state = closeclaw_common::PlanState::new();
        child_plan_state.plan_file_path = plan_file_path;
        child_plan_state.phase = PlanPhase::FinalPlan;
        child_plan_state.step_selection = plan_meta.as_ref().and_then(|m| m.step_selection.clone());
        let _ = child_plan_state.transition_status(PlanStatus::Confirmed);
        let _ = child_plan_state.transition_status(PlanStatus::Executing);
        sm.set_plan_state(&new_session_id, child_plan_state).await;

        // Set new session to Auto Mode.
        sm.set_session_mode(&new_session_id, SessionMode::Auto)
            .await;
        sm.set_pending_mode_transition(&new_session_id, ModeTransition::ExitPlan)
            .await;

        // Push mode switch notification to the new session.
        let mode_msg = PendingMessage::with_role(
            format!("approval-mode-{}", chrono::Utc::now().timestamp_millis()),
            "✅ Plan approved, entering Auto Mode (new session)".to_string(),
            "assistant".to_string(),
        );
        if let Err(e) = sm.push_pending_message(&new_session_id, mode_msg).await {
            tracing::warn!(
                session_id = %new_session_id,
                error = %e,
                "failed to push mode switch notification to new session"
            );
        }
    }

    /// Handle same-session execution path: transition to Auto Mode.
    async fn handle_same_session_path(
        sm: &Arc<dyn SessionLookup>,
        session_id: &str,
        plan_state: &mut closeclaw_common::PlanState,
        plan_meta: &Option<PlanExecMetadata>,
    ) {
        let _ = plan_state.transition_status(PlanStatus::Confirmed);
        let _ = plan_state.transition_status(PlanStatus::Executing);
        Self::update_plan_file_executing(&plan_state.plan_file_path).await;
        plan_state.phase = PlanPhase::FinalPlan;
        plan_state.step_selection = plan_meta.as_ref().and_then(|m| m.step_selection.clone());
        sm.set_plan_state(session_id, plan_state.clone()).await;
        sm.set_session_mode(session_id, SessionMode::Auto).await;
        sm.set_pending_mode_transition(session_id, ModeTransition::ExitPlan)
            .await;
        let mode_msg = PendingMessage::with_role(
            format!("approval-mode-{}", chrono::Utc::now().timestamp_millis()),
            "✅ Plan approved, entering Auto Mode".to_string(),
            "assistant".to_string(),
        );
        if let Err(e) = sm.push_pending_message(session_id, mode_msg).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to push mode switch notification"
            );
        }
    }

    /// Update plan file status to Executing (blocking, spawn-safe).
    async fn update_plan_file_executing(plan_file_path: &str) {
        if !std::path::Path::new(plan_file_path).exists() {
            return;
        }
        let pf_path = plan_file_path.to_string();
        let file_path = plan_file_path.to_string();
        let result = tokio::task::spawn_blocking(move || {
            if let Err(e) = plan_file::update_plan_status(&pf_path, &PlanStatus::Executing) {
                tracing::warn!(
                    plan_file = %file_path,
                    error = %e,
                    "failed to update plan file status"
                );
            }
        });
        if let Err(e) = result.await {
            tracing::warn!(error = %e, "spawn_blocking for plan file update panicked");
        }
    }
}

#[cfg(test)]
mod tests;
