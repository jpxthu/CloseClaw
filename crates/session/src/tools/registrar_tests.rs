//! Tests for [`SessionToolsRegistrar`].
//!
//! Covers:
//! - register correctly registers 4 tools to ToolRegistry
//! - tool names match design doc: sessions_spawn, sessions_steer, sessions_kill, sessions_yield
//! - tool group is "sessions"
//! - duplicate registration triggers Conflict error
//! - priority is 2

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_common::tool_registry::{
    RegistryError, ToolBox, ToolDescriptor, ToolRegistrar, ToolRegistrarError, ToolRegistry,
    ToolRegistryQuery,
};

use super::{LateBoundSessionManagerOps, SessionManagerOps, SessionToolsRegistrar};

// ---------------------------------------------------------------------------
// Mock ToolRegistry for testing
// ---------------------------------------------------------------------------

struct MockToolRegistry {
    tools: tokio::sync::RwLock<HashMap<String, Arc<dyn closeclaw_common::tool_trait::Tool>>>,
    owners: tokio::sync::RwLock<HashMap<String, String>>,
    frozen: std::sync::atomic::AtomicBool,
}

impl MockToolRegistry {
    fn new() -> Self {
        Self {
            tools: tokio::sync::RwLock::new(HashMap::new()),
            owners: tokio::sync::RwLock::new(HashMap::new()),
            frozen: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl ToolRegistry for MockToolRegistry {
    async fn register_any(
        &self,
        tool: Box<dyn std::any::Any + Send + Sync>,
        registrar_name: &str,
    ) -> Result<(), RegistryError> {
        let ToolBox(arc_tool) = *tool
            .downcast::<ToolBox>()
            .map_err(|_| RegistryError::Internal("register_any expected ToolBox".to_string()))?;
        let name = (*arc_tool).name().to_string();

        if self.frozen.load(std::sync::atomic::Ordering::Acquire) {
            return Err(RegistryError::Frozen);
        }

        let mut guard = self.tools.write().await;
        if guard.contains_key(&name) {
            let owners = self.owners.read().await;
            let original = owners.get(&name).cloned().unwrap_or_default();
            drop(guard);
            drop(owners);
            return Err(RegistryError::Conflict {
                tool: name,
                registrar: original,
                attempting: registrar_name.to_string(),
            });
        }
        guard.insert(name.clone(), arc_tool);
        drop(guard);

        let mut owners = self.owners.write().await;
        owners.insert(name, registrar_name.to_string());
        Ok(())
    }

    fn freeze(&self) {
        self.frozen
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn is_frozen(&self) -> bool {
        self.frozen.load(std::sync::atomic::Ordering::Acquire)
    }

    async fn build_index(&self) -> String {
        String::new()
    }
}

#[async_trait]
impl ToolRegistryQuery for MockToolRegistry {
    async fn list_tool_names(&self) -> Vec<String> {
        self.tools.read().await.keys().cloned().collect()
    }

    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<ToolDescriptor> {
        vec![]
    }

    async fn has_tool(&self, name: &str) -> bool {
        self.tools.read().await.contains_key(name)
    }

    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// Mock dependencies
// ---------------------------------------------------------------------------

struct MockSessionManagerOps;

#[async_trait]
impl SessionManagerOps for MockSessionManagerOps {
    async fn create_child_session(
        &self,
        _config: &closeclaw_config::agents::ResolvedAgentConfig,
        _parent_session_id: &str,
        _depth: u32,
        _task: &str,
        _light_context: bool,
        _workspace: Option<&str>,
        _mode: super::super::spawn::SpawnMode,
        _fork: bool,
        _allowed_tools: Option<Vec<String>>,
        _model_override: Option<&str>,
        _parent_subagents_model: Option<&str>,
        _max_spawn_depth: u32,
        _spawn_timeout: Option<u64>,
        _label: Option<&str>,
        _prompt_template_prefix: Option<&str>,
    ) -> Result<String, String> {
        Ok("mock-session-id".to_string())
    }

    async fn validate_child_ownership(
        &self,
        _parent_id: &str,
        _child_id: &str,
    ) -> Option<super::super::spawn::ChildSessionInfo> {
        None
    }

    async fn steer_child(&self, _child_id: &str, _task: &str) -> Result<(), String> {
        Ok(())
    }

    async fn kill_child(&self, _parent_id: &str, _child_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
        Some("mock-chat-id".to_string())
    }

    async fn get_session_depth(&self, _session_id: &str) -> Option<u32> {
        Some(0)
    }

    async fn start_yield_timeout(
        self: Arc<Self>,
        _session_id: &str,
        _agent_id: &str,
        _timeout_secs: Option<u64>,
    ) {
    }
}

struct MockSpawnValidator;

#[async_trait]
impl closeclaw_config::spawn_validation::SpawnValidator for MockSpawnValidator {
    async fn validate_spawn(
        &self,
        _parent_session_id: &str,
        _target_agent_id: Option<&str>,
    ) -> Result<
        closeclaw_config::spawn_validation::SpawnValidationResult,
        closeclaw_config::spawn_validation::SpawnError,
    > {
        Err(closeclaw_config::spawn_validation::SpawnError::AgentIdRequired)
    }
}

struct MockAgentConfigLookup;

#[async_trait]
impl closeclaw_agent::AgentConfigLookup for MockAgentConfigLookup {
    async fn lookup_agent_config(
        &self,
        _agent_id: &str,
    ) -> Option<closeclaw_agent::AgentConfigInfo> {
        None
    }
}

/// Build a mock approval flow that auto-approves.
fn mock_approval_flow() -> closeclaw_common::permission_types::SharedApprovalSubmission {
    use closeclaw_common::permission_types::ApprovalSubmission;
    struct AutoApproveApproval;
    impl ApprovalSubmission for AutoApproveApproval {
        fn submit_inter_agent_denial(
            &self,
            _caller: &closeclaw_common::permission_types::CallerInfo,
            _from: &str,
            _to: &str,
            _risk_level: closeclaw_common::permission_types::RiskLevel,
            _session_id: &str,
            _is_sub_agent: bool,
        ) -> Option<String> {
            Some("mock-approval-id".to_string())
        }
    }
    Arc::new(tokio::sync::Mutex::new(AutoApproveApproval))
}

/// Build a mock permission engine.
fn mock_permission_engine() -> closeclaw_common::permission_types::SharedPermissionEvaluator {
    use closeclaw_common::permission_types::PermissionEvaluator;
    struct AllowAllPermission;
    #[async_trait]
    impl PermissionEvaluator for AllowAllPermission {
        async fn evaluate_inter_agent(
            &self,
            _from: &str,
            _to: &str,
        ) -> closeclaw_common::permission_types::PermissionEvalResponse {
            closeclaw_common::permission_types::PermissionEvalResponse::Allowed
        }
    }
    Arc::new(AllowAllPermission)
}

fn make_registrar() -> SessionToolsRegistrar {
    let late_bound = Arc::new(LateBoundSessionManagerOps::new());
    assert!(late_bound.set(Arc::new(MockSessionManagerOps)).is_ok());
    SessionToolsRegistrar::new(
        Arc::new(MockSpawnValidator),
        late_bound,
        Arc::new(MockAgentConfigLookup),
        mock_permission_engine(),
        mock_approval_flow(),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `SessionToolsRegistrar` priority must be 2.
#[tokio::test]
async fn test_priority_is_2() {
    let registrar = make_registrar();
    assert_eq!(registrar.priority(), 2);
}

/// `SessionToolsRegistrar::register` registers exactly 4 tools with the
/// correct names into the ToolRegistry.
#[tokio::test]
async fn test_registers_four_tools() {
    let registry = MockToolRegistry::new();
    let registrar = make_registrar();

    registrar
        .register(&registry as &dyn ToolRegistry)
        .await
        .expect("register should succeed");

    let mut names = registry.list_tool_names().await;
    names.sort();
    assert_eq!(
        names,
        vec![
            "sessions_kill",
            "sessions_spawn",
            "sessions_steer",
            "sessions_yield",
        ]
    );
}

/// Tool group for all session tools is "sessions".
#[tokio::test]
async fn test_tool_group_is_sessions() {
    let registry = MockToolRegistry::new();
    let registrar = make_registrar();

    registrar
        .register(&registry as &dyn ToolRegistry)
        .await
        .expect("register should succeed");

    let tools = registry.tools.read().await;
    for (name, tool) in tools.iter() {
        assert_eq!(
            tool.group(),
            "sessions",
            "tool `{name}` should belong to group `sessions`"
        );
    }
}

/// Calling `register` twice triggers a Conflict error on the second call
/// because the tools are already registered.
#[tokio::test]
async fn test_duplicate_registration_triggers_conflict() {
    let registry = MockToolRegistry::new();
    let registrar = make_registrar();

    // First registration succeeds.
    registrar
        .register(&registry as &dyn ToolRegistry)
        .await
        .expect("first register should succeed");

    // Second registration must fail with Conflict.
    let result = registrar.register(&registry as &dyn ToolRegistry).await;
    assert!(result.is_err(), "second register should fail with Conflict");
    match result.unwrap_err() {
        ToolRegistrarError::Conflict {
            tool,
            registrar: _,
            attempting,
        } => {
            // The conflict tool should be one of the 4 session tools.
            assert!(
                [
                    "sessions_spawn",
                    "sessions_steer",
                    "sessions_kill",
                    "sessions_yield",
                ]
                .contains(&tool.as_str()),
                "conflicting tool should be a session tool, got `{tool}`"
            );
            assert_eq!(attempting, "SessionToolsRegistrar");
        }
        other => panic!("expected Conflict error, got {:?}", other),
    }
}

/// Verify that all 4 tool names are exactly as specified in the design doc.
#[tokio::test]
async fn test_tool_names_match_design_doc() {
    let registry = MockToolRegistry::new();
    let registrar = make_registrar();

    registrar
        .register(&registry as &dyn ToolRegistry)
        .await
        .expect("register should succeed");

    let expected = [
        "sessions_spawn",
        "sessions_steer",
        "sessions_kill",
        "sessions_yield",
    ];
    for name in &expected {
        assert!(
            registry.has_tool(name).await,
            "registry should contain tool `{name}`"
        );
    }
    assert_eq!(
        registry.list_tool_names().await.len(),
        4,
        "registry should contain exactly 4 tools"
    );
}
