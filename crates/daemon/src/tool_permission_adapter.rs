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
//! without → `evaluate(request, None)`. Request / response / risk /
//! direction types are mapped bidirectionally; the `RiskLevel` mapping
//! mirrors `trait_adapters.rs`.

use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{
    Caller, MessageDirection, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_permission::is_config_file_path;
use closeclaw_tools::permission_port::{
    PermCaller, PermMessageDirection, PermRequestBody, PermRiskLevel, PermVerdict,
    ToolPermissionCheck,
};
use tokio::sync::Mutex as TokioMutex;

/// Map a tools mirror message direction to the permission-side enum.
fn message_direction_to_permission(direction: &PermMessageDirection) -> MessageDirection {
    match direction {
        PermMessageDirection::Send => MessageDirection::Send,
        PermMessageDirection::Receive => MessageDirection::Receive,
        PermMessageDirection::Both => MessageDirection::Both,
    }
}

/// Map a permission-side risk level to the tools mirror.
fn risk_level_to_perm(level: RiskLevel) -> PermRiskLevel {
    match level {
        RiskLevel::Low => PermRiskLevel::Low,
        RiskLevel::Medium => PermRiskLevel::Medium,
        RiskLevel::High => PermRiskLevel::High,
        RiskLevel::Critical => PermRiskLevel::Critical,
    }
}

/// Map a tools mirror risk level to the permission-side enum.
fn risk_level_to_permission(level: PermRiskLevel) -> RiskLevel {
    match level {
        PermRiskLevel::Low => RiskLevel::Low,
        PermRiskLevel::Medium => RiskLevel::Medium,
        PermRiskLevel::High => RiskLevel::High,
        PermRiskLevel::Critical => RiskLevel::Critical,
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
            risk_level: risk_level_to_perm(risk_level),
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
        let agent_perms = self.config_manager.agent_permissions();
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
            let engine = self.permission_engine.read().await;
            let response = engine
                .evaluate_with_chain(
                    upgraded,
                    self.session_manager.as_ref(),
                    sid,
                    agent_perms.as_ref(),
                )
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
            &body_to_permission(body),
            risk_level_to_permission(risk_level),
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
mod tests {
    use super::*;

    // ── Enum mapping（正常路径：全变体覆盖） ─────────────────────────

    #[test]
    fn risk_level_mapping_roundtrips_all_variants() {
        for (perm, expected) in [
            (RiskLevel::Low, PermRiskLevel::Low),
            (RiskLevel::Medium, PermRiskLevel::Medium),
            (RiskLevel::High, PermRiskLevel::High),
            (RiskLevel::Critical, PermRiskLevel::Critical),
        ] {
            assert_eq!(risk_level_to_perm(perm), expected);
            assert_eq!(risk_level_to_permission(expected), perm);
        }
    }

    #[test]
    fn message_direction_mapping_covers_all_variants() {
        for (permission_dir, perm_dir) in [
            (MessageDirection::Send, PermMessageDirection::Send),
            (MessageDirection::Receive, PermMessageDirection::Receive),
            (MessageDirection::Both, PermMessageDirection::Both),
        ] {
            assert_eq!(message_direction_to_permission(&perm_dir), permission_dir);
        }
    }

    // ── Body mapping（字段逐项保留） ─────────────────────────────────

    #[test]
    fn body_mapping_preserves_all_fields_for_every_variant() {
        let cases: Vec<(PermRequestBody, PermissionRequestBody)> = vec![
            (
                PermRequestBody::ToolCall {
                    agent: "a".into(),
                    skill: "bash".into(),
                    method: "call".into(),
                },
                PermissionRequestBody::ToolCall {
                    agent: "a".into(),
                    skill: "bash".into(),
                    method: "call".into(),
                },
            ),
            (
                PermRequestBody::FileOp {
                    agent: "a".into(),
                    path: "/tmp/x".into(),
                    op: "read".into(),
                },
                PermissionRequestBody::FileOp {
                    agent: "a".into(),
                    path: "/tmp/x".into(),
                    op: "read".into(),
                },
            ),
            (
                PermRequestBody::MessageSend {
                    agent: "a".into(),
                    direction: PermMessageDirection::Send,
                    target: "chat_1".into(),
                },
                PermissionRequestBody::MessageSend {
                    agent: "a".into(),
                    direction: MessageDirection::Send,
                    target: "chat_1".into(),
                },
            ),
            (
                PermRequestBody::ConfigWrite {
                    agent: "a".into(),
                    config_file: "config.yaml".into(),
                },
                PermissionRequestBody::ConfigWrite {
                    agent: "a".into(),
                    config_file: "config.yaml".into(),
                },
            ),
            (
                PermRequestBody::NetOp {
                    agent: "a".into(),
                    host: "example.com".into(),
                    port: 443,
                },
                PermissionRequestBody::NetOp {
                    agent: "a".into(),
                    host: "example.com".into(),
                    port: 443,
                },
            ),
            (
                PermRequestBody::CommandExec {
                    agent: "a".into(),
                    cmd: "echo".into(),
                    args: vec!["hi".into()],
                },
                PermissionRequestBody::CommandExec {
                    agent: "a".into(),
                    cmd: "echo".into(),
                    args: vec!["hi".into()],
                },
            ),
        ];
        for (tools_body, expected) in cases {
            // Field preservation is observable through the permission-side
            // dimension + agent accessors plus serde round-trip equality.
            let mapped = body_to_permission(&tools_body);
            assert_eq!(
                serde_json::to_value(&mapped).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            assert_eq!(mapped.dimension_name(), expected.dimension_name());
        }
    }

    // ── Verdict mapping（错误/边界：Allowed、Denied 各形态） ─────────

    #[test]
    fn verdict_mapping_preserves_denied_fields_and_drops_unused() {
        let verdict = verdict_from_permission(PermissionResponse::Allowed {
            token: "tok".into(),
            context_modifier: Some("modifier".into()),
        });
        assert_eq!(verdict, PermVerdict::Allowed);

        let verdict = verdict_from_permission(PermissionResponse::Denied {
            reason: "no rule".into(),
            rule: "r1".into(),
            risk_level: RiskLevel::High,
            approval_request_id: None,
        });
        assert_eq!(
            verdict,
            PermVerdict::Denied {
                reason: "no rule".into(),
                risk_level: PermRiskLevel::High,
                approval_request_id: None,
            }
        );

        let verdict = verdict_from_permission(PermissionResponse::Denied {
            reason: "already queued".into(),
            rule: "r2".into(),
            risk_level: RiskLevel::Critical,
            approval_request_id: Some("req-1".into()),
        });
        assert_eq!(
            verdict,
            PermVerdict::Denied {
                reason: "already queued".into(),
                risk_level: PermRiskLevel::Critical,
                approval_request_id: Some("req-1".into()),
            }
        );
    }

    // ── Config-file classification against a real ConfigManager ──────

    #[tokio::test]
    async fn is_config_file_classifies_config_tree_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_manager = Arc::new(ConfigManager::new(tmp.path().to_path_buf()).unwrap());
        let session_manager = test_session_manager();
        let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
            Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
            Arc::new(|_| {}),
            Arc::new(|_: &str| {}),
            tokio::runtime::Handle::current(),
            closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
            tmp.path().to_path_buf(),
            closeclaw_permission::RuleSet::default(),
        )));
        let adapter = ToolPermissionAdapter::new(
            Arc::new(tokio::sync::RwLock::new(
                PermissionEngine::new_with_default_data_root(
                    closeclaw_permission::RuleSet::default(),
                ),
            )),
            session_manager,
            config_manager,
            approval_flow,
        );
        let data_root = tmp.path().to_path_buf();
        let inside = data_root
            .join("agents/a1/permissions.json")
            .to_string_lossy()
            .into_owned();
        let outside = tmp
            .path()
            .join("workspaces/a1/file.txt")
            .to_string_lossy()
            .into_owned();
        assert!(adapter.is_config_file(&inside));
        assert!(!adapter.is_config_file(&outside));
    }

    // ── Sub-agent decision（空/不存在会话的安全默认） ────────────────

    #[tokio::test]
    async fn is_session_sub_agent_defaults_false_for_empty_or_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_manager = test_session_manager();
        let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
            Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
            Arc::new(|_| {}),
            Arc::new(|_: &str| {}),
            tokio::runtime::Handle::current(),
            closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
            tmp.path().to_path_buf(),
            closeclaw_permission::RuleSet::default(),
        )));
        let adapter = ToolPermissionAdapter::new(
            Arc::new(tokio::sync::RwLock::new(
                PermissionEngine::new_with_default_data_root(
                    closeclaw_permission::RuleSet::default(),
                ),
            )),
            Arc::clone(&session_manager),
            Arc::new(ConfigManager::new(tmp.path().to_path_buf()).unwrap()),
            approval_flow,
        );
        assert!(!adapter.is_session_sub_agent("").await);
        assert!(!adapter.is_session_sub_agent("does-not-exist").await);
    }

    #[tokio::test]
    async fn factory_returns_trait_object() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_manager = test_session_manager();
        let approval_flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
            Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
            Arc::new(|_| {}),
            Arc::new(|_: &str| {}),
            tokio::runtime::Handle::current(),
            closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
            tmp.path().to_path_buf(),
            closeclaw_permission::RuleSet::default(),
        )));
        let port: Arc<dyn ToolPermissionCheck> = tool_permission_check(
            Arc::new(tokio::sync::RwLock::new(
                PermissionEngine::new_with_default_data_root(
                    closeclaw_permission::RuleSet::default(),
                ),
            )),
            session_manager,
            Arc::new(ConfigManager::new(tmp.path().to_path_buf()).unwrap()),
            approval_flow,
        );
        assert!(!port.is_config_file("/tmp/regular/file.txt"));
    }

    fn test_session_manager() -> Arc<SessionManager> {
        Arc::new(SessionManager::new(
            &closeclaw_gateway::GatewayConfig::default(),
            None,
            None,
            closeclaw_session::persistence::ReasoningLevel::default(),
        ))
    }
}
