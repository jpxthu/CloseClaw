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
//! If either level returns `Denied`, the denial is routed through the
//! approval flow (via the [`ToolPermissionCheck`] port for the
//! submission).
//!
//! All engine / session / config interaction goes through the
//! tools-owned port; the composition root (daemon) supplies the
//! production implementation wrapping the real permission engine.

use crate::debug_log::{emit_tool_event, ToolsDebugLogContext, ToolsEmitEventParams};
#[cfg(test)]
use crate::permission_port::PermMessageDirection;
use crate::permission_port::{
    PermCaller, PermRequestBody, PermRiskLevel, PermVerdict, ToolPermissionCheck,
};
use crate::{ToolCallError, ToolResult};

use std::sync::Arc;

use crate::builtin::approval_utils;

/// Bundled permission port shared across all built-in tools.
///
/// Replaces the former concrete `(PermissionEngine, SessionManager,
/// ConfigManager, ApprovalFlow)` tuple with the tools-owned
/// [`ToolPermissionCheck`] port; the composition root (daemon) supplies
/// the production implementation.
pub type PermDeps = Arc<dyn ToolPermissionCheck>;

/// Result of a command-level permission check.
#[derive(Debug)]
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

/// Context for routing a `Denied` permission verdict through the
/// approval flow.
///
/// Bundles the submission context shared by [`route_denial`] and
/// [`route_command_denial`] so both take three arguments instead of
/// seven.
pub(crate) struct DenialRequest<'a> {
    pub(crate) caller: &'a PermCaller,
    pub(crate) body: &'a PermRequestBody,
    pub(crate) risk_level: PermRiskLevel,
    pub(crate) session_id: &'a str,
    pub(crate) is_sub_agent: bool,
}

impl<'a> DenialRequest<'a> {
    /// Bundle the submission context for one denied response.
    pub(crate) fn new(
        caller: &'a PermCaller,
        body: &'a PermRequestBody,
        risk_level: PermRiskLevel,
        session_id: &'a str,
        is_sub_agent: bool,
    ) -> Self {
        Self {
            caller,
            body,
            risk_level,
            session_id,
            is_sub_agent,
        }
    }
}

/// Route a `Denied` verdict through the approval flow.
///
/// On success returns `Ok(Some(ToolResult))` with approval-pending status.
/// If the approval flow rejects the submission (sub-agent / duplicate),
/// returns `Err(PermissionDenied)`.
async fn route_denial(
    verdict: &PermVerdict,
    request: DenialRequest<'_>,
    port: &PermDeps,
) -> Result<Option<ToolResult>, ToolCallError> {
    let (reason, existing_request_id) = match verdict {
        PermVerdict::Denied {
            reason,
            approval_request_id,
            ..
        } => (reason.clone(), approval_request_id.clone()),
        _ => return Ok(None),
    };
    // If the engine already submitted to the approval flow, use that
    // request ID directly instead of re-submitting.
    if let Some(request_id) = existing_request_id {
        return Ok(Some(ToolResult {
            data: approval_utils::build_approval_pending(request_id),
            new_messages: vec![],
            context_modifier: None,
        }));
    }
    if let Some(request_id) = port
        .submit_denial(
            request.caller,
            request.body,
            request.risk_level,
            request.session_id,
            request.is_sub_agent,
        )
        .await
    {
        return Ok(Some(ToolResult {
            data: approval_utils::build_approval_pending(request_id),
            new_messages: vec![],
            context_modifier: None,
        }));
    }
    Err(ToolCallError::PermissionDenied(reason))
}

/// Route a `Denied` verdict for command-level checks.
///
/// Returns `PendingApproval` if the approval flow accepted the request,
/// or `Denied` if the flow rejected it (caller should sandbox the command).
async fn route_command_denial(
    verdict: &PermVerdict,
    request: DenialRequest<'_>,
    port: &PermDeps,
) -> CommandPermissionResult {
    let (reason, existing_request_id) = match verdict {
        PermVerdict::Denied {
            reason,
            approval_request_id,
            ..
        } => (reason.clone(), approval_request_id.clone()),
        _ => return CommandPermissionResult::Permitted,
    };
    // If the engine already submitted to the approval flow, use that
    // request ID directly instead of re-submitting.
    if let Some(request_id) = existing_request_id {
        return CommandPermissionResult::PendingApproval(ToolResult {
            data: approval_utils::build_approval_pending(request_id),
            new_messages: vec![],
            context_modifier: None,
        });
    }
    if let Some(request_id) = port
        .submit_denial(
            request.caller,
            request.body,
            request.risk_level,
            request.session_id,
            request.is_sub_agent,
        )
        .await
    {
        return CommandPermissionResult::PendingApproval(ToolResult {
            data: approval_utils::build_approval_pending(request_id),
            new_messages: vec![],
            context_modifier: None,
        });
    }
    CommandPermissionResult::Denied(reason)
}

/// Evaluate one permission request through the port and route a denial
/// through the approval flow.
///
/// Shared tail of every first/second-level check: builds the caller
/// (user id filled in by the port when the session sender resolves),
/// evaluates, and on `Denied` resolves sub-agent status and routes.
async fn evaluate_and_route(
    port: &PermDeps,
    ctx: &crate::ToolContext,
    body: PermRequestBody,
) -> Result<Option<ToolResult>, ToolCallError> {
    let mut caller = PermCaller {
        user_id: String::new(),
        agent: ctx.agent_id.clone(),
    };
    let verdict = port
        .evaluate(ctx.session_id.as_deref(), &mut caller, &body)
        .await;
    match &verdict {
        PermVerdict::Allowed => Ok(None),
        PermVerdict::Denied { risk_level, .. } => {
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = port.is_session_sub_agent(sid).await;
            route_denial(
                &verdict,
                DenialRequest::new(&caller, &body, *risk_level, sid, is_sub_agent),
                port,
            )
            .await
        }
    }
}

/// First-level check: verify the agent is allowed to invoke the given tool.
///
/// Constructs a `ToolCall` permission request and evaluates it.
/// Returns `Ok(None)` when allowed, `Ok(Some(result))` on approval-pending,
/// or `Err` on denial.
///
/// When `debug_ctx` is provided, emits a `tool.permission` event at
/// [`LogLevel::Info`] with the check result.
pub(crate) async fn check_tool_permission(
    port: &PermDeps,
    ctx: &crate::ToolContext,
    skill: &str,
    method: &str,
    debug_ctx: Option<ToolsDebugLogContext<'_>>,
) -> Result<Option<ToolResult>, ToolCallError> {
    let body = PermRequestBody::ToolCall {
        agent: ctx.agent_id.clone(),
        skill: skill.to_string(),
        method: method.to_string(),
    };
    let result = evaluate_and_route(port, ctx, body).await;
    if let Some(dc) = debug_ctx {
        let permitted = result.as_ref().ok().and_then(|r| r.as_ref()).is_none();
        emit_tool_event(ToolsEmitEventParams {
            ctx: dc,
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "tools",
            event_type: "tool.permission",
            payload: serde_json::json!({
                "dimension": "tool_call",
                "skill": skill,
                "method": method,
                "permitted": permitted,
            }),
            parent: None,
        });
    }
    result
}

/// Second-level check for file operations (FileOp dimension).
///
/// Validates read/write access to the given path.  Callers pass
/// `op = "read"` or `op = "write"` to distinguish the two cases.
///
/// When `debug_ctx` is provided, emits a `tool.permission` event.
pub(crate) async fn check_file_op_permission(
    port: &PermDeps,
    ctx: &crate::ToolContext,
    path: &str,
    op: &str,
    debug_ctx: Option<ToolsDebugLogContext<'_>>,
) -> Result<Option<ToolResult>, ToolCallError> {
    let body = PermRequestBody::FileOp {
        agent: ctx.agent_id.clone(),
        path: path.to_string(),
        op: op.to_string(),
    };
    let result = evaluate_and_route(port, ctx, body).await;
    if let Some(dc) = debug_ctx {
        let permitted = result.as_ref().ok().and_then(|r| r.as_ref()).is_none();
        emit_tool_event(ToolsEmitEventParams {
            ctx: dc,
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "tools",
            event_type: "tool.permission",
            payload: serde_json::json!({
                "dimension": "file_op",
                "path": path,
                "op": op,
                "permitted": permitted,
            }),
            parent: None,
        });
    }
    result
}

/// Second-level check for message operations (Message dimension).
///
/// Validates whether the agent is allowed to send/receive messages
/// in the given direction to/from the specified target.
#[cfg(test)]
pub(crate) async fn check_message_permission(
    port: &PermDeps,
    ctx: &crate::ToolContext,
    direction: PermMessageDirection,
    target: &str,
) -> Result<Option<ToolResult>, ToolCallError> {
    let body = PermRequestBody::MessageSend {
        agent: ctx.agent_id.clone(),
        direction,
        target: target.to_string(),
    };
    evaluate_and_route(port, ctx, body).await
}

/// Second-level check for config write operations (ConfigWrite dimension).
///
/// Validates whether the agent is allowed to write the given config file.
pub(crate) async fn check_config_write_permission(
    port: &PermDeps,
    ctx: &crate::ToolContext,
    config_file: &str,
) -> Result<Option<ToolResult>, ToolCallError> {
    let body = PermRequestBody::ConfigWrite {
        agent: ctx.agent_id.clone(),
        config_file: config_file.to_string(),
    };
    evaluate_and_route(port, ctx, body).await
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
    port: &PermDeps,
    ctx: &crate::ToolContext,
    host: &str,
    port_number: u16,
) -> Result<Option<ToolResult>, ToolCallError> {
    let body = PermRequestBody::NetOp {
        agent: ctx.agent_id.clone(),
        host: host.to_string(),
        port: port_number,
    };
    evaluate_and_route(port, ctx, body).await
}

/// Second-level check for command execution (CommandExec dimension).
///
/// Validates whether the given command and arguments are permitted.
/// Returns a three-way result:
/// - `Permitted` — command is allowed
/// - `PendingApproval` — approval flow accepted the request
/// - `Denied` — permission denied, command should be sandboxed
///
/// When `debug_ctx` is provided, emits a `tool.permission` event.
pub(crate) async fn check_command_permission(
    port: &PermDeps,
    ctx: &crate::ToolContext,
    cmd: &str,
    args: &[String],
    debug_ctx: Option<ToolsDebugLogContext<'_>>,
) -> CommandPermissionResult {
    let body = PermRequestBody::CommandExec {
        agent: ctx.agent_id.clone(),
        cmd: cmd.to_string(),
        args: args.to_vec(),
    };
    let mut caller = PermCaller {
        user_id: String::new(),
        agent: ctx.agent_id.clone(),
    };
    let verdict = port
        .evaluate(ctx.session_id.as_deref(), &mut caller, &body)
        .await;
    let result = match &verdict {
        PermVerdict::Allowed => CommandPermissionResult::Permitted,
        PermVerdict::Denied { risk_level, .. } => {
            let sid = ctx.session_id.as_deref().unwrap_or("");
            let is_sub_agent = port.is_session_sub_agent(sid).await;
            route_command_denial(
                &verdict,
                DenialRequest::new(&caller, &body, *risk_level, sid, is_sub_agent),
                port,
            )
            .await
        }
    };
    if let Some(dc) = debug_ctx {
        let permitted = matches!(result, CommandPermissionResult::Permitted);
        emit_tool_event(ToolsEmitEventParams {
            ctx: dc,
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "tools",
            event_type: "tool.permission",
            payload: serde_json::json!({
                "dimension": "command_exec",
                "cmd": cmd,
                "permitted": permitted,
            }),
            parent: None,
        });
    }
    result
}

#[cfg(test)]
#[path = "permission_check_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "permission_check_subagent_tests.rs"]
mod subagent_tests;

#[cfg(test)]
#[path = "permission_check_route_tests.rs"]
mod route_tests;
