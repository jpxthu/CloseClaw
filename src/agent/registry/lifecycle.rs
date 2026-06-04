//! Lifecycle methods for AgentRegistry

use super::{Agent, AgentProcess, AgentRegistry, RegistryError, RegistryResult};
use tracing::{debug, error, info};

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
        bootstrap_minimal: bool,
        workspace_dir: Option<&str>,
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

        let process = match AgentProcess::spawn(
            agent_binary_path,
            &agent_id,
            bootstrap_minimal,
            workspace_dir,
        )
        .await
        {
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

        info!(agent_id = %agent_id, "agent spawned successfully");
        Ok(agent)
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
        debug!(agent_id = %id, "agent stopped");
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
}

#[cfg(test)]
mod tests {
    use crate::agent::registry::create_registry;

    #[tokio::test]
    async fn test_register_agent() {
        let registry = create_registry(30);
        let agent = registry
            .register("test-agent".to_string(), None)
            .await
            .unwrap();
        assert_eq!(agent.name, "test-agent");
        // Step 1.1 removed `Agent::state`; the registry no longer
        // tracks state, so no state assertion here.
    }

    #[tokio::test]
    async fn test_remove_agent() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        let removed = registry.remove(&agent.id).await.unwrap();
        assert_eq!(removed.id, agent.id);
        assert!(registry.get(&agent.id).await.is_err());
    }
    // Note: `test_update_state` / `test_invalid_state_transition` were
    // removed because `update_state()` is gone. Step 1.3 cleanup is a
    // no-op for these tests.
}
