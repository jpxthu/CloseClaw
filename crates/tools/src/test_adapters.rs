//! Test-only trait adapters for cross-crate permission abstractions.
//!
//! Mirrors `system_prompt::test_adapters`. The tools crate cannot depend
//! on `closeclaw-daemon` or `closeclaw-system-prompt` (which depend on
//! this crate), so these adapters are duplicated here for tests that wire
//! up the tool registrars via `SessionToolsRegistrar`.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_common::permission_types::{
    ApprovalSubmission, CallerInfo, PermissionEvalResponse, PermissionEvaluator, RiskLevel,
};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::agent_permissions_bridge::ConfigAgentPermissionsProviderAdapter;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_risk::assess_risk_level;
use closeclaw_permission::engine::engine_risk::RiskLevel as EngineRiskLevel;
use closeclaw_permission::engine::engine_types::{
    Caller, MessageDirection, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_permission::is_config_file_path;
use closeclaw_permission::PermissionEngine;

use crate::permission_port::{
    PermCaller, PermMessageDirection, PermRequestBody, PermRiskLevel, PermVerdict,
    ToolPermissionCheck,
};

/// Wrapper around `Arc<tokio::sync::RwLock<PermissionEngine>>` implementing
/// [`PermissionEvaluator`].
pub struct PermissionEngineAdapter(pub Arc<tokio::sync::RwLock<PermissionEngine>>);

#[async_trait]
impl PermissionEvaluator for PermissionEngineAdapter {
    async fn evaluate_inter_agent(&self, from: &str, to: &str) -> PermissionEvalResponse {
        let body = PermissionRequestBody::InterAgentMsg {
            from: from.to_string(),
            to: to.to_string(),
        };
        let engine = self.0.read().await;
        match engine.evaluate(PermissionRequest::Bare(body), None) {
            PermissionResponse::Allowed { .. } => PermissionEvalResponse::Allowed,
            PermissionResponse::Denied { reason, .. } => {
                let risk_level = assess_risk_level(&PermissionRequestBody::InterAgentMsg {
                    from: from.to_string(),
                    to: to.to_string(),
                });
                PermissionEvalResponse::Denied {
                    reason,
                    risk_level: map_risk_level(risk_level),
                }
            }
        }
    }
}

/// Wrapper around `Arc<tokio::sync::Mutex<ApprovalFlow>>` implementing
/// [`ApprovalSubmission`].
pub struct ApprovalFlowAdapter(pub Arc<tokio::sync::Mutex<ApprovalFlow>>);

impl ApprovalSubmission for ApprovalFlowAdapter {
    fn submit_inter_agent_denial(
        &self,
        caller: &CallerInfo,
        from: &str,
        to: &str,
        risk_level: RiskLevel,
        session_id: &str,
        is_sub_agent: bool,
    ) -> Option<String> {
        let permission_caller = Caller {
            user_id: caller.user_id.clone(),
            agent: caller.agent.clone(),
        };
        let body = PermissionRequestBody::InterAgentMsg {
            from: from.to_string(),
            to: to.to_string(),
        };
        let mut flow = self.0.blocking_lock();
        flow.submit_denial(
            &permission_caller,
            &body,
            map_risk_level_to_permission(risk_level),
            session_id,
            is_sub_agent,
        )
    }
}

/// Map permission crate's `RiskLevel` to common crate's `RiskLevel`.
fn map_risk_level(level: EngineRiskLevel) -> RiskLevel {
    match level {
        EngineRiskLevel::Low => RiskLevel::Low,
        EngineRiskLevel::Medium => RiskLevel::Medium,
        EngineRiskLevel::High => RiskLevel::High,
        EngineRiskLevel::Critical => RiskLevel::Critical,
    }
}

/// Map common crate's `RiskLevel` to permission crate's `RiskLevel`.
fn map_risk_level_to_permission(level: RiskLevel) -> EngineRiskLevel {
    match level {
        RiskLevel::Low => EngineRiskLevel::Low,
        RiskLevel::Medium => EngineRiskLevel::Medium,
        RiskLevel::High => EngineRiskLevel::High,
        RiskLevel::Critical => EngineRiskLevel::Critical,
    }
}

/// Test adapter bundling the real permission engine, session manager,
/// config manager, and approval flow behind the tools-owned
/// [`ToolPermissionCheck`] port.
///
/// Mirrors the daemon-side production adapter
/// (`tool_permission_adapter.rs`); duplicated here because the tools
/// crate cannot depend on `closeclaw-daemon`. Evaluation replicates the
/// former tools-side `evaluate_permission`: with a session → resolve
/// sender, upgrade Bare → WithCaller, `evaluate_with_chain`; without →
/// `evaluate(request, None)`.
pub struct ToolPermissionCheckAdapter {
    /// Real permission engine.
    pub engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    /// Session manager for sender resolution and sub-agent depth.
    pub session_manager: Arc<SessionManager>,
    /// Config manager for agent permissions and the config root.
    pub config_manager: Arc<ConfigManager>,
    /// Approval flow for denial submission.
    pub approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
}

/// Unified test constructor: bundle the real permission components into
/// a [`ToolPermissionCheckAdapter`] behind the tools-owned
/// [`ToolPermissionCheck`] port ([`crate::permission_check::PermDeps`]).
pub(crate) fn real_permission_port(
    engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    session_manager: Arc<SessionManager>,
    config_manager: Arc<ConfigManager>,
    approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
) -> crate::permission_check::PermDeps {
    Arc::new(ToolPermissionCheckAdapter {
        engine,
        session_manager,
        config_manager,
        approval_flow,
    })
}

fn map_body_to_permission(body: &PermRequestBody) -> PermissionRequestBody {
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
            direction: map_message_direction_to_permission(direction),
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

fn map_message_direction_to_permission(direction: &PermMessageDirection) -> MessageDirection {
    match direction {
        PermMessageDirection::Send => MessageDirection::Send,
        PermMessageDirection::Receive => MessageDirection::Receive,
        PermMessageDirection::Both => MessageDirection::Both,
    }
}

/// Map the permission engine's `RiskLevel` to the tools mirror
/// [`PermRiskLevel`] (kept distinct from the common↔permission mappers
/// above, which cover the inter-agent evaluation surface).
fn map_engine_risk_to_perm(level: EngineRiskLevel) -> PermRiskLevel {
    match level {
        EngineRiskLevel::Low => PermRiskLevel::Low,
        EngineRiskLevel::Medium => PermRiskLevel::Medium,
        EngineRiskLevel::High => PermRiskLevel::High,
        EngineRiskLevel::Critical => PermRiskLevel::Critical,
    }
}

/// Map the tools mirror [`PermRiskLevel`] to the permission engine's
/// `RiskLevel`.
fn map_perm_risk_to_engine(level: PermRiskLevel) -> EngineRiskLevel {
    match level {
        PermRiskLevel::Low => EngineRiskLevel::Low,
        PermRiskLevel::Medium => EngineRiskLevel::Medium,
        PermRiskLevel::High => EngineRiskLevel::High,
        PermRiskLevel::Critical => EngineRiskLevel::Critical,
    }
}

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
            risk_level: map_engine_risk_to_perm(risk_level),
            approval_request_id,
        },
    }
}

#[async_trait]
impl ToolPermissionCheck for ToolPermissionCheckAdapter {
    async fn evaluate(
        &self,
        session_id: Option<&str>,
        caller: &mut PermCaller,
        body: &PermRequestBody,
    ) -> PermVerdict {
        let request = PermissionRequest::Bare(map_body_to_permission(body));
        let agent_perms =
            ConfigAgentPermissionsProviderAdapter::new(self.config_manager.agent_permissions());
        if let Some(sid) = session_id {
            // Resolve the real user_id from the session checkpoint.
            let user_id = self.session_manager.get_sender_id(sid).await;
            if let Some(ref uid) = user_id {
                caller.user_id = uid.clone();
            }
            // Upgrade Bare → WithCaller when user_id is available.
            let upgraded = match user_id {
                Some(ref uid) => {
                    let engine_caller = Caller {
                        user_id: uid.clone(),
                        agent: request.agent_id().to_string(),
                    };
                    request.with_caller(engine_caller)
                }
                None => request,
            };
            let engine = self.engine.read().await;
            let response = engine
                .evaluate_with_chain(upgraded, self.session_manager.as_ref(), sid, &agent_perms)
                .await;
            verdict_from_permission(response)
        } else {
            let response = self.engine.read().await.evaluate(request, None);
            verdict_from_permission(response)
        }
    }

    async fn submit_denial(
        &self,
        caller: &PermCaller,
        body: &PermRequestBody,
        risk_level: PermRiskLevel,
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
            &map_body_to_permission(body),
            map_perm_risk_to_engine(risk_level),
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

/// Test adapter mapping a `ConfigManager` onto the session crate's
/// narrow `SpawnBudgetLookup` port.
///
/// Mirrors the daemon-side production implementation; duplicated here
/// because the tools crate cannot depend on `closeclaw-daemon`. Test
/// fixtures inject agent configs into `ConfigManager::agents` directly.
pub struct ConfigSpawnBudgetLookupAdapter(pub Arc<ConfigManager>);

#[async_trait]
impl closeclaw_session::spawn::controller::SpawnBudgetLookup for ConfigSpawnBudgetLookupAdapter {
    async fn spawn_budget(
        &self,
        agent_id: &str,
    ) -> Option<closeclaw_session::spawn::controller::AgentSpawnBudget> {
        use closeclaw_session::spawn::controller::AgentSpawnBudget;
        let agents = self.0.agents();
        let sc = &agents.get(agent_id)?.subagents;
        Some(AgentSpawnBudget {
            max_spawn_depth: sc.max_spawn_depth,
            max_children: sc.max_children,
            allow_agents: sc.allow_agents.clone(),
            require_agent_id: sc.require_agent_id,
            timeout: sc.timeout,
            timeout_warning: sc.timeout_warning,
            timeout_notify_interval_ratio: sc.timeout_notify_interval_ratio,
        })
    }

    async fn spawn_target_config(
        &self,
        agent_id: &str,
    ) -> Option<closeclaw_session::spawn::controller::SpawnTargetAgentConfig> {
        use closeclaw_session::spawn::controller::SpawnTargetAgentConfig;
        let agents = self.0.agents();
        let cfg = agents.get(agent_id)?;
        Some(SpawnTargetAgentConfig {
            id: cfg.id.clone(),
            model: cfg.model.clone(),
            workspace: cfg.workspace.clone(),
            skills: cfg.skills.clone(),
            tools: cfg.tools.clone(),
            hooks: cfg.hooks.clone(),
        })
    }
}
