//! Test-support adapter: maps a `ConfigManager` fixture onto the
//! session crate's narrow [`SpawnBudgetLookup`] port.
//!
//! Spawn-controller test fixtures inject agent configs into
//! `ConfigManager::agents` directly; this adapter lets the controller
//! consume them after it switched to the injected budget-lookup port.
//! Production wiring uses the daemon-side `ConfigSpawnBudgetLookup`.

use std::sync::Arc;

use closeclaw_config::ConfigManager;
use closeclaw_session::spawn::controller::{
    AgentSpawnBudget, SpawnBudgetLookup, SpawnTargetAgentConfig,
};

/// Build a config-backed [`SpawnBudgetLookup`] over the given manager.
pub(crate) fn config_spawn_budget_lookup(cm: &Arc<ConfigManager>) -> Arc<dyn SpawnBudgetLookup> {
    Arc::new(ConfigManagerSpawnBudgetLookup(Arc::clone(cm)))
}

struct ConfigManagerSpawnBudgetLookup(Arc<ConfigManager>);

#[async_trait::async_trait]
impl SpawnBudgetLookup for ConfigManagerSpawnBudgetLookup {
    async fn spawn_budget(&self, agent_id: &str) -> Option<AgentSpawnBudget> {
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

    async fn spawn_target_config(&self, agent_id: &str) -> Option<SpawnTargetAgentConfig> {
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
