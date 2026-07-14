//! Unit tests for SpawnController::validate().
//!
//! Covers the validation step sequence aligned with the design doc
//! (docs/design/agent/agent-spawn.md §Spawn 控制流程 ①-⑥):
//!
//! - Normal path: requireAgentId=false, no agentId → fallback to parent
//! - Error path: requireAgentId=true, no agentId → AgentIdRequired
//! - Error path: depth budget = 0 → DepthExceeded
//! - Normal path: depth budget > 0, valid target → success

use std::sync::Arc;

use closeclaw_common::{PermissionChecker, SpawnPermissionError};
use closeclaw_config::agents::SubagentsConfig;
use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};
use closeclaw_config::ConfigManager;

use super::controller::{SpawnContext, SpawnController};
use super::error::SpawnError;

// ── Mock implementations ───────────────────────────────────────────────

/// Mock SpawnContext for unit tests. Configurable per-test via fields.
struct MockSpawnContext {
    active_children: usize,
    chat_id: Option<String>,
    effective_budget: Option<u32>,
}

impl MockSpawnContext {
    fn with_budget(budget: Option<u32>) -> Self {
        Self {
            active_children: 0,
            chat_id: Some("parent-agent".to_string()),
            effective_budget: budget,
        }
    }
}

#[async_trait::async_trait]
impl SpawnContext for MockSpawnContext {
    async fn active_children_count(&self, _parent_session_id: &str) -> usize {
        self.active_children
    }

    async fn chat_id(&self, _session_id: &str) -> Option<String> {
        self.chat_id.clone()
    }

    async fn sender_id(&self, _session_id: &str) -> Option<String> {
        None
    }

    async fn effective_max_spawn_depth(&self, _session_id: &str) -> Option<u32> {
        self.effective_budget
    }
}

/// Mock PermissionChecker that always allows spawn.
struct AllowAllPermissionChecker;

#[async_trait::async_trait]
impl PermissionChecker for AllowAllPermissionChecker {
    async fn validate_spawn_permission(
        &self,
        _child_agent_id: &str,
        _parent_session_id: &str,
    ) -> Result<(), SpawnPermissionError> {
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Build a ResolvedAgentConfig with the given subagent settings.
fn make_agent_config(id: &str, subagents: SubagentsConfig) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: None,
        workspace: None,
        agent_dir: None,
        bootstrap_mode: closeclaw_common::BootstrapMode::Full,
        skills: vec!["*".to_string()],
        tools: vec!["*".to_string()],
        disallowed_tools: vec![],
        subagents,
        memory: Default::default(),
        hooks: Vec::new(),
        source: ConfigSource::User,
    }
}

/// Create a ConfigManager with the given agents pre-loaded.
fn make_config_manager(agents: Vec<ResolvedAgentConfig>) -> Arc<ConfigManager> {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mgr = ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager");
    {
        let mut map = mgr.agents.write().expect("poisoned");
        for agent in agents {
            map.insert(agent.id.clone(), agent);
        }
    }
    Arc::new(mgr)
}

fn make_controller(
    config_manager: Arc<ConfigManager>,
    context: Arc<dyn SpawnContext>,
) -> SpawnController {
    SpawnController::new(config_manager, context, Arc::new(AllowAllPermissionChecker))
}

// ── Tests ──────────────────────────────────────────────────────────────

/// Normal path: requireAgentId=false, no agentId provided → fallback to
/// parent agent ID ("parent-agent"), passes whitelist (wildcard).
#[tokio::test]
async fn test_require_agent_id_false_fallback_to_parent() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", subagents);
    let config_manager = make_config_manager(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", None).await;

    let result = result.expect("validate should succeed when requireAgentId=false");
    assert_eq!(result.config.id, "parent-agent");
}

/// Error path: requireAgentId=true, no agentId provided → reject
/// AgentIdRequired immediately (design doc §Spawn 控制流程 ③).
#[tokio::test]
async fn test_require_agent_id_true_rejects_without_agent_id() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(true),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", subagents);
    let config_manager = make_config_manager(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", None).await;

    match result {
        Err(SpawnError::AgentIdRequired) => {} // expected
        other => panic!("expected AgentIdRequired, got {:?}", other),
    }
}

/// Error path: effective budget = 0 → DepthExceeded (design doc §Spawn
/// 控制流程 ①).
#[tokio::test]
async fn test_depth_budget_zero_rejects() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(1),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", subagents);
    let config_manager = make_config_manager(vec![parent_config]);
    // Mock context returns budget = 0.
    let context = Arc::new(MockSpawnContext::with_budget(Some(0)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", None).await;

    match result {
        Err(SpawnError::DepthExceeded { current: 1, max: 0 }) => {} // expected
        other => panic!("expected DepthExceeded, got {:?}", other),
    }
}

/// Normal path: depth budget > 0, valid target agent → success.
#[tokio::test]
async fn test_valid_spawn_with_budget() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let target_subagents = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(2),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", subagents);
    let target_config = make_agent_config("child-agent", target_subagents);
    let config_manager = make_config_manager(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", Some("child-agent")).await;

    let result = result.expect("validate should succeed");
    assert_eq!(result.config.id, "child-agent");
    // effective_max_spawn_depth = min(target.max_spawn_depth=2, parent_budget-1=1) = 1
    assert_eq!(result.effective_max_spawn_depth, 1);
}

/// Whitelist rejection: target agent not in allowAgents → AgentNotAllowed.
#[tokio::test]
async fn test_whitelist_rejects_unknown_agent() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["allowed-agent".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", subagents);
    let config_manager = make_config_manager(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller
        .validate("session-1", Some("unknown-agent"))
        .await;

    match result {
        Err(SpawnError::AgentNotAllowed { agent_id }) => {
            assert_eq!(agent_id, "unknown-agent");
        }
        other => panic!("expected AgentNotAllowed, got {:?}", other),
    }
}

/// requireAgentId=true, explicit agentId provided → passes requireAgentId
/// check, proceeds to whitelist check (which passes with wildcard).
#[tokio::test]
async fn test_require_agent_id_true_with_explicit_agent_id() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(true),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let target_subagents = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(2),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", subagents);
    let target_config = make_agent_config("child-agent", target_subagents);
    let config_manager = make_config_manager(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", Some("child-agent")).await;

    let result = result.expect("validate should succeed with explicit agentId");
    assert_eq!(result.config.id, "child-agent");
}

// ═══════════════════════════════════════════════════════════════════════
// Timeout priority chain tests (design doc §timeout)
// ═══════════════════════════════════════════════════════════════════════

/// Target agent has subagents.timeout=60 → spawn_timeout=Some(60).
#[tokio::test]
async fn test_timeout_from_target_agent_config() {
    let parent_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let target_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(2),
        max_children: Some(5),
        timeout: Some(60),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", parent_sub);
    let target_config = make_agent_config("child-agent", target_sub);
    let config_manager = make_config_manager(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.spawn_timeout, Some(60));
}

/// Target agent has no timeout → spawn_timeout=None.
#[tokio::test]
async fn test_timeout_none_when_target_has_no_config() {
    let parent_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let target_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(2),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", parent_sub);
    let target_config = make_agent_config("child-agent", target_sub);
    let config_manager = make_config_manager(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.spawn_timeout, None);
}

/// Target agent timeout=0 → passthrough as Some(0).
#[tokio::test]
async fn test_timeout_zero_passthrough() {
    let parent_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let target_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(2),
        max_children: Some(5),
        timeout: Some(0),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", parent_sub);
    let target_config = make_agent_config("child-agent", target_sub);
    let config_manager = make_config_manager(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(config_manager, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.spawn_timeout, Some(0));
}
