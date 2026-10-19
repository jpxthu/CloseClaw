//! Production tools-side [`ToolPermissionAdapter`] implementation for
//! the daemon composition root.
//!
//! Wraps the real `PermissionEngine` + `SessionManager` +
//! `ConfigManager` + `ApprovalFlow` behind
//! `closeclaw_tools::permission_port::ToolPermissionCheck` so builtin
//! tools run two-level permission checks without depending on
//! `closeclaw-permission` / `closeclaw-gateway` / `closeclaw-config`.
//!
//! `evaluate` replicates the former tools-side `evaluate_permission`:
//! with a session → resolve the sender (`get_sender_id`), upgrade
//! Bare → WithCaller, `evaluate_with_chain` with agent permissions;
//! without → `evaluate(request, None)`. An empty checkpoint sender is
//! normalized to "no sender bound" (gateway-aligned, mirroring
//! `session_handler_tool_dispatch.rs`) and takes the Bare path —
//! `owner_id` is only injected for a non-empty bound sender. Request /
//! response / direction types are mapped bidirectionally; risk levels
//! are shared via `closeclaw_common::permission_types::RiskLevel`.

use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_common::permission_types::RiskLevel;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Caller, MessageDirection, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_permission::is_config_file_path;
use closeclaw_tools::permission_port::{
    PermCaller, PermMessageDirection, PermRequestBody, PermVerdict, ToolPermissionCheck,
};
use tokio::sync::Mutex as TokioMutex;

use crate::agent_permissions_adapter::AgentPermissionsAdapter;

/// Map a tools mirror message direction to the permission-side enum.
fn message_direction_to_permission(direction: &PermMessageDirection) -> MessageDirection {
    match direction {
        PermMessageDirection::Send => MessageDirection::Send,
        PermMessageDirection::Receive => MessageDirection::Receive,
        PermMessageDirection::Both => MessageDirection::Both,
    }
}

/// Map a tools mirror request body to the permission-side enum,
/// preserving every field.
fn body_to_permission(body: &PermRequestBody) -> PermissionRequestBody {
    match body {
        PermRequestBody::ToolCall {
            agent,
            skill,
            method,
        } => PermissionRequestBody::ToolCall {
            agent: agent.clone(),
            skill: skill.clone(),
            method: method.clone(),
        },
        PermRequestBody::FileOp { agent, path, op } => PermissionRequestBody::FileOp {
            agent: agent.clone(),
            path: path.clone(),
            op: op.clone(),
        },
        PermRequestBody::MessageSend {
            agent,
            direction,
            target,
        } => PermissionRequestBody::MessageSend {
            agent: agent.clone(),
            direction: message_direction_to_permission(direction),
            target: target.clone(),
        },
        PermRequestBody::ConfigWrite { agent, config_file } => PermissionRequestBody::ConfigWrite {
            agent: agent.clone(),
            config_file: config_file.clone(),
        },
        PermRequestBody::NetOp { agent, host, port } => PermissionRequestBody::NetOp {
            agent: agent.clone(),
            host: host.clone(),
            port: *port,
        },
        PermRequestBody::CommandExec { agent, cmd, args } => PermissionRequestBody::CommandExec {
            agent: agent.clone(),
            cmd: cmd.clone(),
            args: args.clone(),
        },
    }
}

/// Map a permission-side response to the tools mirror verdict.
fn verdict_from_permission(response: PermissionResponse) -> PermVerdict {
    match response {
        PermissionResponse::Allowed { .. } => PermVerdict::Allowed,
        PermissionResponse::Denied {
            reason,
            risk_level,
            approval_request_id,
            ..
        } => PermVerdict::Denied {
            reason,
            risk_level,
            approval_request_id,
        },
    }
}

/// Production tools-side permission check adapter wrapping the real
/// permission engine, session manager, config manager, and approval flow.
pub struct ToolPermissionAdapter {
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    session_manager: Arc<SessionManager>,
    config_manager: Arc<ConfigManager>,
    approval_flow: Arc<TokioMutex<ApprovalFlow>>,
}

impl ToolPermissionAdapter {
    /// Wrap the given engine, session manager, config manager, and
    /// approval flow.
    pub fn new(
        permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
        approval_flow: Arc<TokioMutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            permission_engine,
            session_manager,
            config_manager,
            approval_flow,
        }
    }
}

#[async_trait]
impl ToolPermissionCheck for ToolPermissionAdapter {
    async fn evaluate(
        &self,
        session_id: Option<&str>,
        caller: &mut PermCaller,
        body: &PermRequestBody,
    ) -> PermVerdict {
        let request = PermissionRequest::Bare(body_to_permission(body));
        let agent_perms = AgentPermissionsAdapter::new(self.config_manager.agent_permissions());
        if let Some(sid) = session_id {
            // Resolve the real user_id from the session checkpoint.
            //
            // Empty-sender normalization (gateway-aligned, mirrors
            // `session_handler_tool_dispatch.rs`'s `owner_id` guard):
            // a checkpoint `sender_id` of `Some("")` is treated as "no
            // sender bound" — the empty string must never reach
            // `owner_id`, or `caller.user_id == owner_id` (`"" == ""`)
            // would fire the Owner shortcut for an unidentified
            // caller. Normalized `Some("")` takes the same Bare path
            // as `None`: the engine request carries an empty
            // `caller.user_id` and no Owner is bound, matching the
            // gateway injection point's semantics.
            let user_id = self
                .session_manager
                .get_sender_id(sid)
                .await
                .filter(|uid| !uid.is_empty());
            if let Some(ref uid) = user_id {
                caller.user_id = uid.clone();
            }
            // Upgrade Bare → WithCaller when user_id is available.
            //
            // Safety-semantics filing (关联 issue #3792): on the tools
            // dispatch path the caller IS the session-bound Owner — the
            // checkpoint sender resolved above feeds both
            // `caller.user_id` and `owner_id`, so the engine's Owner
            // shortcut (skip the User phase and the auto-mode
            // dangerous-operation review; design
            // `docs/design/permission/approval-workflow.md` §Owner 审批权力)
            // always applies when a sender is bound. It is therefore
            // intended that User-dimension defaults and the auto-mode
            // review are unreachable on this path: the session sender
            // was bound from the authentic inbound message at session
            // creation, and these tool calls execute on that session's
            // behalf. The safeguards remain in force for callers that
            // are not provably the session Owner — `None` and empty
            // checkpoint senders (both normalized to "no sender
            // bound", Bare request) → nobody qualifies for the Owner
            // shortcut.
            let upgraded = match user_id {
                Some(ref uid) => {
                    let engine_caller = Caller {
                        user_id: uid.clone(),
                        agent: request.agent_id().to_string(),
                    };
                    request
                        .with_caller(engine_caller)
                        .with_owner_id(user_id.clone())
                }
                None => request,
            };
            let engine = self.permission_engine.read().await;
            let response = engine
                .evaluate_with_chain(upgraded, self.session_manager.as_ref(), sid, &agent_perms)
                .await;
            verdict_from_permission(response)
        } else {
            let response = self.permission_engine.read().await.evaluate(request, None);
            verdict_from_permission(response)
        }
    }

    async fn submit_denial(
        &self,
        caller: &PermCaller,
        body: &PermRequestBody,
        risk_level: RiskLevel,
        session_id: &str,
        is_sub_agent: bool,
    ) -> Option<String> {
        let permission_caller = Caller {
            user_id: caller.user_id.clone(),
            agent: caller.agent.clone(),
        };
        let mut flow = self.approval_flow.lock().await;
        flow.submit_denial(
            &permission_caller,
            &body_to_permission(body),
            risk_level,
            session_id,
            is_sub_agent,
        )
    }

    async fn is_session_sub_agent(&self, session_id: &str) -> bool {
        if session_id.is_empty() {
            return false;
        }
        self.session_manager
            .get_session_depth(session_id)
            .await
            .is_some_and(|depth| depth > 0)
    }

    fn is_config_file(&self, path: &str) -> bool {
        let data_root = self.config_manager.config_dir();
        is_config_file_path(data_root, path)
    }
}

/// Build the production tools-side permission check port as
/// `Arc<dyn ToolPermissionCheck>`.
pub fn tool_permission_check(
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    session_manager: Arc<SessionManager>,
    config_manager: Arc<ConfigManager>,
    approval_flow: Arc<TokioMutex<ApprovalFlow>>,
) -> Arc<dyn ToolPermissionCheck> {
    Arc::new(ToolPermissionAdapter::new(
        permission_engine,
        session_manager,
        config_manager,
        approval_flow,
    ))
}

#[cfg(test)]
#[path = "tool_permission_adapter_tests.rs"]
mod tests;
