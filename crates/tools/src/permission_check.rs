//! Two-level permission check helpers for built-in tools.
//!
//! Extracts the common permission-checking pattern from [`BashTool`] into
//! reusable functions that all built-in tools can share.  The design doc
//! mandates two levels of checking:
//!
//! 1. **ToolCall** — is the agent allowed to invoke this tool at all?
//! 2. **Domain dimension** — is the specific operation (FileOp / CommandExec)
//!    allowed?
//!
//! If either level returns `Denied`, the denial is routed through
//! [`ApprovalFlow`] for owner approval.

use crate::{ToolCallError, ToolResult};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{
    Caller, MessageDirection, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_permission::is_config_file_path;
use closeclaw_permission::PermissionResponse as PR;

use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::builtin::approval_utils;

type PermEngine = tokio::sync::RwLock<PermissionEngine>;
type ApprovalMutex = TokioMutex<ApprovalFlow>;

/// Bundled permission dependencies shared across all built-in tools.
pub type PermDeps = (
    Arc<PermEngine>,
    Arc<SessionManager>,
    Arc<ConfigManager>,
    Arc<ApprovalMutex>,
);

/// Result of a command-level permission check.
pub(crate) enum CommandPermissionResult {
    /// Command is permitted — execute normally.
    Permitted,
    /// Approval flow accepted the request — return approval-pending to
    /// the caller (do NOT execute the command).
    PendingApproval(ToolResult),
    /// Permission denied and approval flow rejected — the command should
    /// be routed to the sandbox for restricted execution.
    Denied(String),
}

/// Route a `Denied` response through the approval flow.
///
/// On success returns `Ok(Some(ToolResult))` with approval-pending status.
/// If the approval flow rejects the submission (sub-agent / duplicate),
/// returns `Err(PermissionDenied)`.
async fn route_denial(
    response: &PR,
    caller: &Caller,
    body: &PermissionRequestBody,
    risk_level: RiskLevel,
    session_id: &str,
    is_sub_agent: bool,
    approval_flow: &Arc<ApprovalMutex>,
) -> Result<Option<ToolResult>, ToolCallError> {
    let reason = match response {
        PR::Denied { reason, .. } => reason.clone(),
        _ => return Ok(None),
    };
    let mut flow = approval_flow.lock().await;
    if let Some(request_id) = flow.submit_denial(caller, body, risk_level, session_id, is_sub_agent)
    {
        return Ok(Some(ToolResult {
            data: approval_utils::build_approval_pending(request_id),
            new_messages: vec![],
            context_modifier: None,
        }));
    }
    Err(ToolCallError::PermissionDenied(reason))
}

/// Route a `Denied` response for command-level checks.
///
/// Returns `PendingApproval` if the approval flow accepted the request,
/// or `Denied` if the flow rejected it (caller should sandbox the command).
async fn route_command_denial(
    response: &PR,
    caller: &Caller,
    body: &PermissionRequestBody,
    risk_level: RiskLevel,
    session_id: &str,
    is_sub_agent: bool,
    approval_flow: &Arc<ApprovalMutex>,
) -> CommandPermissionResult {
    let reason = match response {
        PR::Denied { reason, .. } => reason.clone(),
        _ => return CommandPermissionResult::Permitted,
    };
    let mut flow = approval_flow.lock().await;
    if let Some(request_id) = flow.submit_denial(caller, body, risk_level, session_id, is_sub_agent)
    {
        return CommandPermissionResult::PendingApproval(ToolResult {
            data: approval_utils::build_approval_pending(request_id),
            new_messages: vec![],
            context_modifier: None,
        });
    }
    CommandPermissionResult::Denied(reason)
}

/// Evaluate a permission request through the engine, optionally using
/// chain evaluation when a session is present.
async fn evaluate_permission(
    perm: &Arc<PermEngine>,
    session_manager: &Arc<SessionManager>,
    config_manager: &Arc<ConfigManager>,
    session_id: Option<&str>,
    request: PermissionRequest,
) -> PermissionResponse {
    let agent_perms = config_manager.agent_permissions();
    if let Some(sid) = session_id {
        let engine = perm.read().await;
        engine
            .evaluate_with_chain(request, session_manager.as_ref(), sid, agent_perms.as_ref())
            .await
    } else {
        perm.read().await.evaluate(request, None)
    }
}

/// Determine whether the given session belongs to a sub-agent (depth > 0).
///
/// Returns `false` when the session id is empty or the depth cannot be
/// resolved (e.g. session not found), which is the safe default.
async fn is_session_sub_agent(session_manager: &Arc<SessionManager>, session_id: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    session_manager
        .get_session_depth(session_id)
        .await
        .is_some_and(|depth| depth > 0)
}

/// Check if a file path targets a config file.
///
/// Uses [`closeclaw_permission::is_config_file_path`] with the data root
/// obtained from [`ConfigManager::config_dir`].  Returns `true` when the
/// path is inside the config directory but outside any workspace.
pub(crate) fn is_config_file(config_manager: &ConfigManager, path: &str) -> bool {
    let data_root = config_manager.config_dir();
    is_config_file_path(data_root, path)
}

/// First-level check: verify the agent is allowed to invoke the given tool.
///
/// Constructs a `ToolCall` permission request and evaluates it.
/// Returns `Ok(None)` when allowed, `Ok(Some(result))` on approval-pending,
/// or `Err` on denial.
pub(crate) async fn check_tool_permission(
    deps: &PermDeps,
    ctx: &crate::ToolContext,
    skill: &str,
    method: &str,
) -> Result<Option<ToolResult>, ToolCallError> {
    let (perm, session_manager, config_manager, approval_flow) = deps;
    let request = PermissionRequest::Bare(PermissionRequestBody::ToolCall {
        agent: ctx.agent_id.clone(),
        skill: skill.to_string(),
        method: method.to_string(),
    });
    let response = evaluate_permission(
        perm,
        session_manager,
        config_manager,
        ctx.session_id.as_deref(),
        request,
    )
    .await;
    match response {
        PR::Allowed { .. } => Ok(None),
        PR::Denied { risk_level, .. } => {
            let caller = Caller {
                user_id: String::new(),
                agent: ctx.agent_id.clone(),
                creator_id: String::new(),
            };
            let body = PermissionRequestBody::ToolCall {
                agent: ctx.agent_id.clone(),
                skill: skill.to_string(),
                method: method.to_string(),
            };
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = is_session_sub_agent(session_manager, sid).await;
            route_denial(
                &response,
                &caller,
                &body,
                risk_level,
                sid,
                is_sub_agent,
                approval_flow,
            )
            .await
        }
    }
}

/// Second-level check for file operations (FileOp dimension).
///
/// Validates read/write access to the given path.  Callers pass
/// `op = "read"` or `op = "write"` to distinguish the two cases.
pub(crate) async fn check_file_op_permission(
    deps: &PermDeps,
    ctx: &crate::ToolContext,
    path: &str,
    op: &str,
) -> Result<Option<ToolResult>, ToolCallError> {
    let (perm, session_manager, config_manager, approval_flow) = deps;
    let request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: ctx.agent_id.clone(),
        path: path.to_string(),
        op: op.to_string(),
    });
    let response = evaluate_permission(
        perm,
        session_manager,
        config_manager,
        ctx.session_id.as_deref(),
        request,
    )
    .await;
    match response {
        PR::Allowed { .. } => Ok(None),
        PR::Denied { risk_level, .. } => {
            let caller = Caller {
                user_id: String::new(),
                agent: ctx.agent_id.clone(),
                creator_id: String::new(),
            };
            let body = PermissionRequestBody::FileOp {
                agent: ctx.agent_id.clone(),
                path: path.to_string(),
                op: op.to_string(),
            };
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = is_session_sub_agent(session_manager, sid).await;
            route_denial(
                &response,
                &caller,
                &body,
                risk_level,
                sid,
                is_sub_agent,
                approval_flow,
            )
            .await
        }
    }
}

/// Second-level check for message operations (Message dimension).
///
/// Validates whether the agent is allowed to send/receive messages
/// in the given direction to/from the specified target.
#[allow(dead_code)]
pub(crate) async fn check_message_permission(
    deps: &PermDeps,
    ctx: &crate::ToolContext,
    direction: MessageDirection,
    target: &str,
) -> Result<Option<ToolResult>, ToolCallError> {
    let (perm, session_manager, config_manager, approval_flow) = deps;
    let request = PermissionRequest::Bare(PermissionRequestBody::MessageSend {
        agent: ctx.agent_id.clone(),
        direction: direction.clone(),
        target: target.to_string(),
    });
    let response = evaluate_permission(
        perm,
        session_manager,
        config_manager,
        ctx.session_id.as_deref(),
        request,
    )
    .await;
    match response {
        PR::Allowed { .. } => Ok(None),
        PR::Denied { risk_level, .. } => {
            let caller = Caller {
                user_id: String::new(),
                agent: ctx.agent_id.clone(),
                creator_id: String::new(),
            };
            let body = PermissionRequestBody::MessageSend {
                agent: ctx.agent_id.clone(),
                direction,
                target: target.to_string(),
            };
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = is_session_sub_agent(session_manager, sid).await;
            route_denial(
                &response,
                &caller,
                &body,
                risk_level,
                sid,
                is_sub_agent,
                approval_flow,
            )
            .await
        }
    }
}

/// Second-level check for config write operations (ConfigWrite dimension).
///
/// Validates whether the agent is allowed to write the given config file.
pub(crate) async fn check_config_write_permission(
    deps: &PermDeps,
    ctx: &crate::ToolContext,
    config_file: &str,
) -> Result<Option<ToolResult>, ToolCallError> {
    let (perm, session_manager, config_manager, approval_flow) = deps;
    let request = PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
        agent: ctx.agent_id.clone(),
        config_file: config_file.to_string(),
    });
    let response = evaluate_permission(
        perm,
        session_manager,
        config_manager,
        ctx.session_id.as_deref(),
        request,
    )
    .await;
    match response {
        PR::Allowed { .. } => Ok(None),
        PR::Denied { risk_level, .. } => {
            let caller = Caller {
                user_id: String::new(),
                agent: ctx.agent_id.clone(),
                creator_id: String::new(),
            };
            let body = PermissionRequestBody::ConfigWrite {
                agent: ctx.agent_id.clone(),
                config_file: config_file.to_string(),
            };
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = is_session_sub_agent(session_manager, sid).await;
            route_denial(
                &response,
                &caller,
                &body,
                risk_level,
                sid,
                is_sub_agent,
                approval_flow,
            )
            .await
        }
    }
}

/// Second-level check for network operations (NetOp dimension).
///
/// Validates whether the agent is allowed to connect to the specified host and port.
///
/// Currently, BashTool's network access is implicitly covered by the command dimension
/// (CommandExec) since network commands like `curl`/`wget` are treated as command execution.
/// This function is reserved for future dedicated network tools that perform direct I/O
/// (e.g., an HTTP client tool) and need explicit network permission checks.
pub async fn check_network_permission(
    deps: &PermDeps,
    ctx: &crate::ToolContext,
    host: &str,
    port: u16,
) -> Result<Option<ToolResult>, ToolCallError> {
    let (perm, session_manager, config_manager, approval_flow) = deps;
    let request = PermissionRequest::Bare(PermissionRequestBody::NetOp {
        agent: ctx.agent_id.clone(),
        host: host.to_string(),
        port,
    });
    let response = evaluate_permission(
        perm,
        session_manager,
        config_manager,
        ctx.session_id.as_deref(),
        request,
    )
    .await;
    match response {
        PR::Allowed { .. } => Ok(None),
        PR::Denied { risk_level, .. } => {
            let caller = Caller {
                user_id: String::new(),
                agent: ctx.agent_id.clone(),
                creator_id: String::new(),
            };
            let body = PermissionRequestBody::NetOp {
                agent: ctx.agent_id.clone(),
                host: host.to_string(),
                port,
            };
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = is_session_sub_agent(session_manager, sid).await;
            route_denial(
                &response,
                &caller,
                &body,
                risk_level,
                sid,
                is_sub_agent,
                approval_flow,
            )
            .await
        }
    }
}

/// Second-level check for command execution (CommandExec dimension).
///
/// Validates whether the given command and arguments are permitted.
/// Returns a three-way result:
/// - `Permitted` — command is allowed
/// - `PendingApproval` — approval flow accepted the request
/// - `Denied` — permission denied, command should be sandboxed
pub(crate) async fn check_command_permission(
    deps: &PermDeps,
    ctx: &crate::ToolContext,
    cmd: &str,
    args: &[String],
) -> CommandPermissionResult {
    let (perm, session_manager, config_manager, approval_flow) = deps;
    let request = PermissionRequest::Bare(PermissionRequestBody::CommandExec {
        agent: ctx.agent_id.clone(),
        cmd: cmd.to_string(),
        args: args.to_vec(),
    });
    let response = evaluate_permission(
        perm,
        session_manager,
        config_manager,
        ctx.session_id.as_deref(),
        request,
    )
    .await;
    match response {
        PR::Allowed { .. } => CommandPermissionResult::Permitted,
        PR::Denied { risk_level, .. } => {
            let caller = Caller {
                user_id: String::new(),
                agent: ctx.agent_id.clone(),
                creator_id: String::new(),
            };
            let body = PermissionRequestBody::CommandExec {
                agent: ctx.agent_id.clone(),
                cmd: cmd.to_string(),
                args: args.to_vec(),
            };
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = is_session_sub_agent(session_manager, sid).await;
            route_command_denial(
                &response,
                &caller,
                &body,
                risk_level,
                sid,
                is_sub_agent,
                approval_flow,
            )
            .await
        }
    }
}

#[cfg(test)]
#[path = "permission_check_tests.rs"]
mod tests;
