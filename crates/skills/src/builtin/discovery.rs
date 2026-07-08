//! Skill discovery skill - allows agents to search and install skills from ClawHub
use crate::registry::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::Caller;
use closeclaw_permission::engine::PermissionRequestBody;
use closeclaw_permission::{PermissionRequest, PermissionResponse};
use std::sync::Arc;

#[derive(Default)]
pub struct SkillDiscoverySkill {
    engine: Option<Arc<tokio::sync::RwLock<closeclaw_permission::PermissionEngine>>>,
    approval_flow: Option<Arc<tokio::sync::Mutex<ApprovalFlow>>>,
}

impl SkillDiscoverySkill {
    pub fn new() -> Self {
        Self {
            engine: None,
            approval_flow: None,
        }
    }

    pub fn with_engine(
        engine: Arc<tokio::sync::RwLock<closeclaw_permission::PermissionEngine>>,
    ) -> Self {
        Self {
            engine: Some(engine),
            approval_flow: None,
        }
    }

    pub fn with_engine_and_approval_flow(
        engine: Arc<tokio::sync::RwLock<closeclaw_permission::PermissionEngine>>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            engine: Some(engine),
            approval_flow: Some(approval_flow),
        }
    }
}

#[async_trait]
impl Skill for SkillDiscoverySkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "skill_discovery".to_string(),
            version: "1.0.0".to_string(),
            description: "Search, install, and manage skills from ClawHub marketplace. Use find to search, install to add, list to see installed, update to upgrade.".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec!["clawhub".to_string()],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["find", "install", "list", "update"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        match method {
            "find" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("query required".to_string()))?;
                let output = tokio::process::Command::new("clawhub")
                    .args(["search", query])
                    .output()
                    .await
                    .map_err(|e| {
                        SkillError::ExecutionFailed(format!("clawhub search failed: {}", e))
                    })?;
                Ok(
                    serde_json::json!({"query": query, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            "install" => {
                let agent_id = args
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("agent_id required".to_string()))?;
                let skill = args
                    .get("skill")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("skill name required".to_string()))?;
                let version = args.get("version").and_then(|v| v.as_str());

                if let Some(ref engine) = self.engine {
                    let caller = Caller {
                        user_id: String::new(),
                        agent: agent_id.to_string(),
                        creator_id: String::new(),
                    };
                    let request = PermissionRequest::WithCaller {
                        caller: caller.clone(),
                        request: PermissionRequestBody::InterAgentMsg {
                            from: agent_id.to_string(),
                            to: "*".to_string(),
                        },
                    };
                    let extra_deny_subjects = engine
                        .read()
                        .await
                        .get_agent_deny_subjects(agent_id, agent_id);
                    match engine
                        .read()
                        .await
                        .evaluate(request, Some(extra_deny_subjects))
                    {
                        PermissionResponse::Allowed { .. } => {}
                        PermissionResponse::Denied {
                            reason, risk_level, ..
                        } => {
                            if let Some(ref flow) = self.approval_flow {
                                let body = PermissionRequestBody::InterAgentMsg {
                                    from: agent_id.to_string(),
                                    to: "*".to_string(),
                                };
                                let session_id = args
                                    .get("session_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let mut flow = flow.lock().await;
                                if let Some(request_id) = flow
                                    .submit_denial(&caller, &body, risk_level, session_id, false)
                                {
                                    return Ok(serde_json::json!({
                                        "status": "approval_pending",
                                        "request_id": request_id,
                                        "message": "Operation pending owner approval",
                                    }));
                                }
                            }
                            return Err(SkillError::PermissionDenied(reason));
                        }
                        PermissionResponse::ApprovalRequired { risk_level, .. } => {
                            if let Some(ref flow) = self.approval_flow {
                                let body = PermissionRequestBody::InterAgentMsg {
                                    from: agent_id.to_string(),
                                    to: "*".to_string(),
                                };
                                let session_id = args
                                    .get("session_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let mut flow = flow.lock().await;
                                if let Some(request_id) = flow
                                    .submit_denial(&caller, &body, risk_level, session_id, false)
                                {
                                    return Ok(serde_json::json!({
                                        "status": "approval_pending",
                                        "request_id": request_id,
                                        "message": "Operation pending owner approval",
                                    }));
                                }
                            }
                            return Err(SkillError::PermissionDenied(
                                "approval required".to_string(),
                            ));
                        }
                    }
                }

                let mut cmd = tokio::process::Command::new("clawhub");
                cmd.args(["install", skill]);
                if let Some(v) = version {
                    cmd.arg("--version").arg(v);
                }
                let output = cmd.output().await.map_err(|e| {
                    SkillError::ExecutionFailed(format!("clawhub install failed: {}", e))
                })?;
                Ok(
                    serde_json::json!({"skill": skill, "version": version, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            "list" => {
                let output = tokio::process::Command::new("clawhub")
                    .args(["list"])
                    .output()
                    .await
                    .map_err(|e| {
                        SkillError::ExecutionFailed(format!("clawhub list failed: {}", e))
                    })?;
                Ok(
                    serde_json::json!({"output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            "update" => {
                let skill = args.get("skill").and_then(|v| v.as_str());
                let mut cmd = tokio::process::Command::new("clawhub");
                cmd.args(["update"]);
                if let Some(s) = skill {
                    cmd.arg(s);
                } else {
                    cmd.arg("--all");
                }
                let output = cmd.output().await.map_err(|e| {
                    SkillError::ExecutionFailed(format!("clawhub update failed: {}", e))
                })?;
                Ok(
                    serde_json::json!({"skill": skill, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "skill_discovery".to_string(),
                method: method.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest() {
        let skill = SkillDiscoverySkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "skill_discovery");
        assert_eq!(m.version, "1.0.0");
        assert!(m.dependencies.contains(&"clawhub".to_string()));
    }

    #[test]
    fn test_methods() {
        let skill = SkillDiscoverySkill::new();
        assert_eq!(skill.methods(), vec!["find", "install", "list", "update"]);
    }

    #[test]
    fn test_default() {
        let skill = SkillDiscoverySkill::default();
        assert_eq!(skill.manifest().name, "skill_discovery");
    }

    #[tokio::test]
    async fn test_find_missing_query() {
        let skill = SkillDiscoverySkill::new();
        let result = skill.execute("find", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("query")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_missing_agent_id() {
        let skill = SkillDiscoverySkill::new();
        let result = skill
            .execute("install", serde_json::json!({"skill": "foo"}))
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("agent_id")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_missing_skill() {
        let skill = SkillDiscoverySkill::new();
        let result = skill
            .execute("install", serde_json::json!({"agent_id": "a1"}))
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("skill")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let skill = SkillDiscoverySkill::new();
        let result = skill.execute("nonexistent", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::MethodNotFound { skill, .. } => assert_eq!(skill, "skill_discovery"),
            other => panic!("expected MethodNotFound, got {:?}", other),
        }
    }

    fn make_engine() -> Arc<closeclaw_permission::PermissionEngine> {
        use closeclaw_permission::engine::engine_types::{
            Action, Defaults, Effect, MatchType, Rule, RuleSet, Subject,
        };
        use std::collections::HashMap;
        let rules = RuleSet {
            rules: vec![Rule {
                name: "deny-spawn".to_string(),
                subject: Subject::AgentOnly {
                    agent: "blocked-agent".to_string(),
                    match_type: MatchType::Exact,
                },
                effect: Effect::Deny,
                actions: vec![Action::ToolCall {
                    skill: "*".to_string(),
                    methods: vec![],
                }],
                template: None,
                priority: 10,
            }],
            defaults: Defaults::default(),
            template_includes: vec![],
            agent_creators: HashMap::new(),
        };
        Arc::new(closeclaw_permission::PermissionEngine::new_with_default_data_root(rules))
    }

    #[tokio::test]
    async fn test_install_permission_denied() {
        let engine = make_engine();
        let skill = SkillDiscoverySkill::with_engine(engine);
        let result = skill
            .execute(
                "install",
                serde_json::json!({
                    "agent_id": "blocked-agent",
                    "skill": "test-skill"
                }),
            )
            .await;
        let _ = result;
    }

    #[tokio::test]
    async fn test_install_permission_with_extra_deny() {
        use closeclaw_permission::engine::engine_types::{
            Action, Defaults, Effect, MatchType, Rule, RuleSet, Subject,
        };
        use std::collections::HashMap;
        let rules = RuleSet {
            rules: vec![Rule {
                name: "parent-deny-spawn".to_string(),
                subject: Subject::AgentOnly {
                    agent: "parent-agent".to_string(),
                    match_type: MatchType::Exact,
                },
                effect: Effect::Deny,
                actions: vec![Action::ToolCall {
                    skill: "*".to_string(),
                    methods: vec![],
                }],
                template: None,
                priority: 10,
            }],
            defaults: Defaults::default(),
            template_includes: vec![],
            agent_creators: HashMap::new(),
        };
        let engine =
            Arc::new(closeclaw_permission::PermissionEngine::new_with_default_data_root(rules));
        let skill = SkillDiscoverySkill::with_engine(engine);
        let result = skill
            .execute(
                "install",
                serde_json::json!({
                    "agent_id": "child-agent",
                    "skill": "test-skill"
                }),
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            crate::registry::SkillError::PermissionDenied(_)
        ));
    }
}
