//! Lifecycle methods for AgentRegistry

use super::{
    Agent, AgentProcess, AgentRegistry, AgentState, AgentStateTransition, CleanupResult,
    RegistryError, RegistryResult, TransitionTrigger,
};
use tracing::{debug, error, info, warn};

impl AgentRegistry {
    /// Register a new agent (creates metadata, doesn't spawn process yet)
    pub async fn register(&self, name: String, parent_id: Option<String>) -> RegistryResult<Agent> {
        let agent = Agent::new(name, parent_id);
        let mut agents = self.agents.write().await;
        let id = agent.id.clone();
        debug!(agent_id = %id, name = %agent.name, "registering new agent");
        agents.insert(id, agent.clone());
        Ok(agent)
    }

    /// Spawn an agent as a child process
    pub async fn spawn(
        &self,
        name: String,
        parent_id: Option<String>,
        agent_binary_path: &str,
    ) -> RegistryResult<Agent> {
        let path = std::path::Path::new(agent_binary_path);
        if !path.is_absolute() {
            return Err(RegistryError::ProcessError(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("agent binary path must be absolute: {}", agent_binary_path),
                )
                .into(),
            ));
        }
        if !path.exists() {
            return Err(RegistryError::ProcessError(
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("agent binary not found: {}", agent_binary_path),
                )
                .into(),
            ));
        }

        let agent = self.register(name, parent_id).await?;
        let agent_id = agent.id.clone();

        let process = match AgentProcess::spawn(agent_binary_path, &agent_id).await {
            Ok(p) => p,
            Err(e) => {
                error!(agent_id = %agent_id, error = %e, "failed to spawn agent process, cleaning up registration");
                let _ = self.remove(&agent_id).await;
                return Err(RegistryError::ProcessError(e));
            }
        };

        {
            let mut processes = self.processes.write().await;
            processes.insert(agent_id.clone(), process);
        }

        self.update_state(&agent_id, AgentState::Running, TransitionTrigger::Scheduler)
            .await?;

        info!(agent_id = %agent_id, "agent spawned successfully");
        Ok(agent)
    }
}

impl AgentRegistry {
    /// Update agent state
    pub async fn update_state(
        &self,
        id: &str,
        new_state: AgentState,
        trigger: TransitionTrigger,
    ) -> RegistryResult<Agent> {
        let mut agents = self.agents.write().await;
        let agent = agents
            .get_mut(id)
            .ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))?;

        if !crate::agent::state::is_valid_transition(&agent.state, &new_state) {
            return Err(RegistryError::InvalidStateTransition(format!(
                "{:?} -> {:?}",
                agent.state, new_state
            )));
        }

        let from_state = agent.state.clone();
        let transition = AgentStateTransition::new(from_state.clone(), new_state.clone(), trigger);
        debug!(
            agent_id = %id,
            from = ?from_state,
            to = ?new_state,
            trigger = ?transition.trigger,
            timestamp = %transition.timestamp,
            "state transition"
        );

        agent.set_state(new_state);
        let _ = transition;
        Ok(agent.clone())
    }

    /// Update heartbeat for an agent
    pub async fn update_heartbeat(&self, id: &str) -> RegistryResult<()> {
        let mut agents = self.agents.write().await;
        let agent = agents
            .get_mut(id)
            .ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))?;
        agent.update_heartbeat();
        Ok(())
    }
}

impl AgentRegistry {
    /// Remove an agent (cleanup process and metadata)
    pub async fn remove(&self, id: &str) -> RegistryResult<Agent> {
        self.kill(id).await.ok();
        {
            let mut processes = self.processes.write().await;
            processes.remove(id);
        }
        let mut agents = self.agents.write().await;
        agents
            .remove(id)
            .ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))
    }

    /// Kill a running agent process
    pub async fn kill(&self, id: &str) -> RegistryResult<()> {
        let process = {
            let processes = self.processes.read().await;
            processes.get(id).cloned()
        };
        if let Some(mut p) = process {
            debug!(agent_id = %id, "killing agent process");
            p.kill().await?;
        }
        match self
            .update_state(id, AgentState::Stopped, TransitionTrigger::SystemShutdown)
            .await
        {
            Ok(_) => debug!(agent_id = %id, "agent stopped"),
            Err(e) => warn!(agent_id = %id, error = %e, "failed to update agent state to Stopped"),
        }
        Ok(())
    }

    /// Send a message to an agent via stdin
    pub async fn send_message(&self, id: &str, message: &str) -> RegistryResult<()> {
        let process = {
            let processes = self.processes.read().await;
            processes.get(id).cloned()
        };
        let mut process = process.ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))?;
        process.send_message(message).await?;
        Ok(())
    }

    /// Check for dead agents and clean them up
    pub async fn cleanup_dead(&self) -> CleanupResult {
        let mut dead_ids = Vec::new();
        {
            let agents = self.agents.read().await;
            for (id, agent) in agents.iter() {
                if agent.is_terminal() {
                    continue;
                }
                if !agent.is_alive(self.heartbeat_timeout_secs) {
                    warn!(agent_id = %id, "agent heartbeat expired, marking for cleanup");
                    dead_ids.push(id.clone());
                }
            }
        }
        let mut cleaned = Vec::new();
        let mut failed = Vec::new();
        for id in &dead_ids {
            match self.remove(id).await {
                Ok(_) => cleaned.push(id.clone()),
                Err(e) => {
                    error!(agent_id = %id, error = %e, "failed to cleanup dead agent");
                    failed.push((id.clone(), e));
                }
            }
        }
        CleanupResult { cleaned, failed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::registry::create_registry;
    use crate::agent::state::SuspendedReason;

    #[tokio::test]
    async fn test_register_agent() {
        let registry = create_registry(30);
        let agent = registry
            .register("test-agent".to_string(), None)
            .await
            .unwrap();
        assert_eq!(agent.name, "test-agent");
        assert!(matches!(agent.state, AgentState::Idle));
    }

    #[tokio::test]
    async fn test_update_state() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        registry
            .update_state(
                &agent.id,
                AgentState::Running,
                TransitionTrigger::UserRequest,
            )
            .await
            .unwrap();
        let updated = registry.get(&agent.id).await.unwrap();
        assert!(matches!(updated.state, AgentState::Running));
    }

    #[tokio::test]
    async fn test_invalid_state_transition() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        registry
            .update_state(
                &agent.id,
                AgentState::Stopped,
                TransitionTrigger::UserRequest,
            )
            .await
            .unwrap();
        let result = registry
            .update_state(
                &agent.id,
                AgentState::Running,
                TransitionTrigger::UserRequest,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_remove_agent() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        let removed = registry.remove(&agent.id).await.unwrap();
        assert_eq!(removed.id, agent.id);
        assert!(registry.get(&agent.id).await.is_err());
    }
}
