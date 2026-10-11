use crate::bash::CommandSandbox;
use crate::permission_check::PermDeps;
use crate::permission_port::{PermCaller, PermRequestBody, PermRiskLevel};
use crate::security::{BashSecurityAnalyzer, ParseResult, TrustLevel};
use crate::{ToolCallError, ToolContext, ToolResult};
use serde_json::Value;

use super::exec::resolve_cwd;

/// Analyze command security and return the parsed result.
///
/// Returns the [`ParseResult`] so the caller can inspect `trust_level`
/// and use the tree-sitter parsed `commands` (argv arrays) for
/// downstream permission checks.
pub(super) fn analyze_security(command: &str) -> Result<ParseResult, ToolCallError> {
    let sec_result = BashSecurityAnalyzer::new()
        .map_err(ToolCallError::ExecutionFailed)?
        .analyze(command);
    Ok(sec_result)
}

/// Submit a trust-level denial to the approval flow and optionally notify the owner.
///
/// Builds a [`PermCaller`] and [`PermRequestBody`] from the tool context, then
/// submits the denial through the permission port. Returns the request ID if the
/// approval flow accepted the request (i.e. owner was notified / queued), or
/// `None` if hard-denied (sub-agent or duplicate).
///
/// P3 note: `user_id` is intentionally left empty here because
/// trust-level routing (malicious/uncertain) occurs before user identity
/// verification — at this point in the pipeline we only know the agent ID,
/// not the Feishu user identity. The approval flow will resolve the actual
/// user identity from the session manager when it processes the request.
async fn submit_trust_level_denial(
    deps: &PermDeps,
    ctx: &ToolContext,
    command: &str,
    risk: PermRiskLevel,
) -> Option<String> {
    let caller = PermCaller {
        user_id: String::new(),
        agent: ctx.agent_id.clone(),
    };
    let body = PermRequestBody::CommandExec {
        agent: ctx.agent_id.clone(),
        cmd: command.to_string(),
        args: vec![],
    };
    let sid = ctx.session_id.as_deref().unwrap_or("");
    let is_sub_agent = deps.is_session_sub_agent(sid).await;
    deps.submit_denial(&caller, &body, risk, sid, is_sub_agent)
        .await
}

/// Execute the BashTool call: parse args, check two-level permissions, run command.
/// Result of trust-level routing.
pub(super) enum TrustDecision {
    /// Command is trusted; proceed with normal permission checks.
    Trusted,
    /// Command was routed to approval flow; caller returns the pending result.
    ApprovalPending(ToolResult),
    /// Command was blocked (malicious or uncertain-rejected).
    Blocked(String),
}

/// Route trust-level decisions (malicious/uncertain) through the approval flow.
pub(super) async fn route_trust_level(
    deps: &PermDeps,
    ctx: &ToolContext,
    command: &str,
    trust_level: TrustLevel,
    reason: Option<String>,
) -> Result<TrustDecision, ToolCallError> {
    match trust_level {
        TrustLevel::Malicious => {
            submit_trust_level_denial(deps, ctx, command, PermRiskLevel::Critical).await;
            Ok(TrustDecision::Blocked(format!(
                "Blocked: malicious command detected — {}",
                reason.unwrap_or_else(|| "unknown reason".into())
            )))
        }
        TrustLevel::Uncertain => {
            let r = reason.unwrap_or_else(|| "untrusted syntax".into());
            if let Some(request_id) =
                submit_trust_level_denial(deps, ctx, command, PermRiskLevel::High).await
            {
                Ok(TrustDecision::ApprovalPending(ToolResult {
                    data: crate::builtin::approval_utils::build_approval_pending(request_id),
                    new_messages: vec![],
                    context_modifier: None,
                }))
            } else {
                Ok(TrustDecision::Blocked(format!(
                    "Blocked: uncertain command — {}",
                    r
                )))
            }
        }
        TrustLevel::Trusted => Ok(TrustDecision::Trusted),
    }
}

/// Resolve working directory and apply sandbox restrictions if needed.
pub(super) async fn prepare_and_sandbox(
    ctx: &ToolContext,
    command: &str,
    args: &Value,
) -> Result<String, ToolCallError> {
    let dangerously_disable = args.get("dangerouslyDisableSandbox") == Some(&Value::Bool(true));
    let cwd = resolve_cwd(args, ctx);
    if !dangerously_disable && CommandSandbox::should_sandbox(command, true) {
        CommandSandbox::apply_sandbox_restrictions(&cwd)?;
    }
    Ok(cwd)
}
