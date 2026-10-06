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

use super::controller::{
    AgentSpawnBudget, SpawnBudgetLookup, SpawnContext, SpawnController, SpawnTargetAgentConfig,
};
use closeclaw_common::{SpawnError, SpawnValidationResult};

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

/// Configurable PermissionChecker mock: `deny_reason` set → deny with that
/// reason, `None` → allow. Lets one mock cover both branches of
/// `check_spawn_permission`.
struct ConfigurablePermissionChecker {
    deny_reason: Option<String>,
}

#[async_trait::async_trait]
impl PermissionChecker for ConfigurablePermissionChecker {
    async fn validate_spawn_permission(
        &self,
        child_agent_id: &str,
        _parent_session_id: &str,
    ) -> Result<(), SpawnPermissionError> {
        match &self.deny_reason {
            None => Ok(()),
            Some(reason) => Err(SpawnPermissionError::Denied {
                agent_id: child_agent_id.to_string(),
                reason: reason.clone(),
            }),
        }
    }
}

/// Local mock replacing the old ConfigManager fixture: holds agent
/// config fixtures and maps them onto the narrow spawn-budget view.
struct MockSpawnBudgetLookup {
    agents: Vec<ResolvedAgentConfig>,
}

#[async_trait::async_trait]
impl SpawnBudgetLookup for MockSpawnBudgetLookup {
    async fn spawn_budget(&self, agent_id: &str) -> Option<AgentSpawnBudget> {
        let sc = &self.agents.iter().find(|a| a.id == agent_id)?.subagents;
        Some(AgentSpawnBudget {
            max_spawn_depth: sc.max_spawn_depth,
            max_children: sc.max_children,
            allow_agents: Some(sc.allow_agents.clone()),
            require_agent_id: sc.require_agent_id,
            timeout: sc.timeout,
            timeout_warning: sc.timeout_warning,
            timeout_notify_interval_ratio: sc.timeout_notify_interval_ratio,
        })
    }

    async fn spawn_target_config(&self, agent_id: &str) -> Option<SpawnTargetAgentConfig> {
        let cfg = self.agents.iter().find(|a| a.id == agent_id)?;
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
        parallel_tool_calls: true,
        memory_configured: false,
        source: ConfigSource::User,
    }
}

/// Create a budget lookup with the given agents pre-loaded.
fn make_budget_lookup(agents: Vec<ResolvedAgentConfig>) -> Arc<dyn SpawnBudgetLookup> {
    Arc::new(MockSpawnBudgetLookup { agents })
}

fn make_controller(
    budget_lookup: Arc<dyn SpawnBudgetLookup>,
    context: Arc<dyn SpawnContext>,
) -> SpawnController {
    SpawnController::new(budget_lookup, context, Arc::new(AllowAllPermissionChecker))
}

fn make_controller_with_checker(
    budget_lookup: Arc<dyn SpawnBudgetLookup>,
    context: Arc<dyn SpawnContext>,
    checker: ConfigurablePermissionChecker,
) -> SpawnController {
    SpawnController::new(budget_lookup, context, Arc::new(checker))
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
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", None).await;

    let result = result.expect("validate should succeed when requireAgentId=false");
    assert_eq!(result.agent_id, "parent-agent");
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
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
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
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    // Mock context returns budget = 0.
    let context = Arc::new(MockSpawnContext::with_budget(Some(0)));

    let controller = make_controller(budget_lookup, context);
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
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;

    let result = result.expect("validate should succeed");
    assert_eq!(result.agent_id, "child-agent");
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
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
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
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;

    let result = result.expect("validate should succeed with explicit agentId");
    assert_eq!(result.agent_id, "child-agent");
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
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.spawn_timeout, Some(60));
}

/// Target agent has no timeout → falls back to global default (172800s = 48h).
#[tokio::test]
async fn test_timeout_global_default_when_target_has_no_config() {
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
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.spawn_timeout, Some(172800));
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
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.spawn_timeout, Some(0));
}

// ═══════════════════════════════════════════════════════════════════════
// timeout_warning priority chain tests (design doc §timeout_warning)
// ═══════════════════════════════════════════════════════════════════════

/// Target agent has subagents.timeout_warning=120 →
/// SpawnValidationResult.timeout_warning_secs=Some(120).
#[tokio::test]
async fn test_timeout_warning_from_target_agent_config() {
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
        timeout_warning: Some(120),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", parent_sub);
    let target_config = make_agent_config("child-agent", target_sub);
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.timeout_warning_secs, Some(120));
}

/// Target agent has no timeout_warning → result is None (legacy mode).
#[tokio::test]
async fn test_timeout_warning_none_when_target_has_no_config() {
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
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.timeout_warning_secs, None);
}

/// Target agent has timeout_notify_interval_ratio=0.8 → resolved correctly.
#[tokio::test]
async fn test_timeout_notify_interval_ratio_from_target_agent_config() {
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
        timeout_warning: Some(90),
        timeout_notify_interval_ratio: Some(0.8),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", parent_sub);
    let target_config = make_agent_config("child-agent", target_sub);
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.timeout_warning_secs, Some(90));
    assert_eq!(result.timeout_notify_interval_ratio, Some(0.8));
}

/// Target agent timeout_warning=0 → passthrough as Some(0).
#[tokio::test]
async fn test_timeout_warning_zero_passthrough() {
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
        timeout_warning: Some(0),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", parent_sub);
    let target_config = make_agent_config("child-agent", target_sub);
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("validate should succeed");
    assert_eq!(result.timeout_warning_secs, Some(0));
}

// ═══════════════════════════════════════════════════════════════════════
// Budget ≤ 0 semantics tests (design doc §Depth 追踪)
// ═══════════════════════════════════════════════════════════════════════

/// Budget=1 allows spawn (child effective budget = min(target, 1-1) = 0).
#[tokio::test]
async fn test_budget_one_allows_spawn() {
    let parent_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(1),
        max_children: Some(5),
        ..Default::default()
    };
    let target_sub = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(1),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", parent_sub);
    let target_config = make_agent_config("child-agent", target_sub);
    let budget_lookup = make_budget_lookup(vec![parent_config, target_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(1)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", Some("child-agent")).await;
    let result = result.expect("budget=1 should allow spawn");
    assert_eq!(result.agent_id, "child-agent");
    // effective = min(target.max_spawn_depth=1, parent_budget-1=0) = 0
    assert_eq!(result.effective_max_spawn_depth, 0);
}

/// Budget=0 blocks spawn (design doc: effective budget ≤ 0 → blocked).
#[tokio::test]
async fn test_budget_zero_blocks_spawn() {
    let subagents = SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    };
    let parent_config = make_agent_config("parent-agent", subagents);
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(0)));

    let controller = make_controller(budget_lookup, context);
    let result = controller.validate("session-1", None).await;

    match result {
        Err(SpawnError::DepthExceeded { current: 1, max: 0 }) => {}
        other => panic!(
            "budget=0 must block spawn with DepthExceeded, got {:?}",
            other
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// check_spawn_permission — internal config re-resolution (two-step separation)
// ═══════════════════════════════════════════════════════════════════════

/// Build a validation result targeting `agent_id`.
fn make_validation(agent_id: &str) -> SpawnValidationResult {
    SpawnValidationResult {
        agent_id: agent_id.to_string(),
        effective_max_spawn_depth: 1,
        spawn_timeout: Some(172800),
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
    }
}

/// Parent subagent settings that permit any target agent.
fn permissive_subagents() -> SubagentsConfig {
    SubagentsConfig {
        require_agent_id: Some(false),
        allow_agents: vec!["*".to_string()],
        max_spawn_depth: Some(3),
        max_children: Some(5),
        ..Default::default()
    }
}

/// Re-resolution failure: the validation's `agent_id` is unknown to the
/// config store → `SpawnError::ConfigNotFound` instead of a permission
/// verdict (the full config is re-resolved inside the session domain).
#[tokio::test]
async fn test_check_spawn_permission_config_not_found() {
    let parent_config = make_agent_config("parent-agent", permissive_subagents());
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));
    let controller = make_controller(budget_lookup, context);

    let validation = make_validation("missing-agent");
    let result = controller
        .check_spawn_permission("session-1", &validation)
        .await;

    match result {
        Err(SpawnError::ConfigNotFound(id)) => assert_eq!(id, "missing-agent"),
        other => panic!("expected ConfigNotFound, got {:?}", other),
    }
}

/// Re-resolution success + checker denial → `SpawnError::Permission`
/// carrying the checker's payload (permission verdict is step 2).
#[tokio::test]
async fn test_check_spawn_permission_denied_propagates() {
    let parent_config = make_agent_config("parent-agent", permissive_subagents());
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));
    let controller = make_controller_with_checker(
        budget_lookup,
        context,
        ConfigurablePermissionChecker {
            deny_reason: Some("blocked by policy".to_string()),
        },
    );

    let validation = make_validation("parent-agent");
    let result = controller
        .check_spawn_permission("session-1", &validation)
        .await;

    match result {
        Err(SpawnError::Permission(SpawnPermissionError::Denied { agent_id, reason })) => {
            assert_eq!(agent_id, "parent-agent");
            assert_eq!(reason, "blocked by policy");
        }
        other => panic!("expected Permission(Denied), got {:?}", other),
    }
}

/// Re-resolution success + checker approval → `Ok(())`.
#[tokio::test]
async fn test_check_spawn_permission_allowed() {
    let parent_config = make_agent_config("parent-agent", permissive_subagents());
    let budget_lookup = make_budget_lookup(vec![parent_config]);
    let context = Arc::new(MockSpawnContext::with_budget(Some(2)));
    let controller = make_controller_with_checker(
        budget_lookup,
        context,
        ConfigurablePermissionChecker { deny_reason: None },
    );

    let validation = make_validation("parent-agent");
    controller
        .check_spawn_permission("session-1", &validation)
        .await
        .expect("allow-all checker should let the spawn through");
}
