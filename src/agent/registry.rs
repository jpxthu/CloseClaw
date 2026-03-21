//! Agent Registry - manages all agent lifecycles
//!
//! Provides centralized management for creating, tracking, and removing agents.

use crate::agent::process::{AgentProcess, AgentProcessHandle};
use crate::agent::{Agent, AgentState};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Errors that can occur in the agent registry
#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("agent already exists: {0}")]
    AgentAlreadyExists(String),
    #[error("invalid state transition: {0}")]
    InvalidStateTransition(String),
    #[error("process error: {0}")]
    ProcessError(#[from] crate::agent::process::ProcessError),
}

/// Result type for registry operations
pub type RegistryResult<T> = Result<T, RegistryError>;

/// Thread-safe agent registry for managing all agent lifecycles
#[derive(Debug)]
pub struct AgentRegistry {
    /// Map of agent ID to agent metadata
    agents: RwLock<HashMap<String, Agent>>,
    /// Map of agent ID to running process handle
    processes: RwLock<HashMap<String, AgentProcessHandle>>,
    /// Heartbeat timeout in seconds
    heartbeat_timeout_secs: i64,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new(30) // Default 30 second heartbeat timeout
    }
}

impl AgentRegistry {
    /// Create a new registry with specified heartbeat timeout
    pub fn new(heartbeat_timeout_secs: i64) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            processes: RwLock::new(HashMap::new()),
            heartbeat_timeout_secs,
        }
    }

    /// Register a new agent (creates metadata, doesn't spawn process yet)
    pub async fn register(&self, name: String, parent_id: Option<String>) -> Agent {
        let agent = Agent::new(name, parent_id);
        let mut agents = self.agents.write().await;
        let id = agent.id.clone();
        debug!(agent_id = %id, name = %agent.name, "registering new agent");
        agents.insert(id, agent.clone());
        agent
    }

    /// Spawn an agent as a child process
    pub async fn spawn(
        &self,
        name: String,
        parent_id: Option<String>,
        agent_binary_path: &str,
    ) -> RegistryResult<Agent> {
        let agent = self.register(name, parent_id).await;
        let agent_id = agent.id.clone();

        // Spawn the process
        let process = AgentProcess::spawn(agent_binary_path, &agent_id)
            .await
            .map_err(|e| {
                // Clean up registration on spawn failure
                error!(agent_id = %agent_id, error = %e, "failed to spawn agent process");
                RegistryError::ProcessError(e)
            })?;

        // Store process handle
        {
            let mut processes = self.processes.write().await;
            processes.insert(agent_id.clone(), process);
        }

        // Update state to Running
        self.update_state(&agent_id, AgentState::Running).await?;

        info!(agent_id = %agent_id, "agent spawned successfully");
        Ok(agent)
    }

    /// Get an agent by ID
    pub async fn get(&self, id: &str) -> RegistryResult<Agent> {
        let agents = self.agents.read().await;
        agents
            .get(id)
            .cloned()
            .ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))
    }

    /// Get an agent by ID, checking if alive (heartbeat)
    pub async fn get_alive(&self, id: &str) -> RegistryResult<Agent> {
        let agent = self.get(id).await?;
        if !agent.is_alive(self.heartbeat_timeout_secs) {
            warn!(agent_id = %id, "agent heartbeat expired");
            return Err(RegistryError::AgentNotFound(id.to_string()));
        }
        Ok(agent)
    }

    /// List all registered agents
    pub async fn list(&self) -> Vec<Agent> {
        let agents = self.agents.read().await;
        agents.values().cloned().collect()
    }

    /// List only alive agents (heartbeat within threshold)
    pub async fn list_alive(&self) -> Vec<Agent> {
        let agents = self.agents.read().await;
        agents
            .values()
            .filter(|a| a.is_alive(self.heartbeat_timeout_secs))
            .cloned()
            .collect()
    }

    /// List agents by state
    pub async fn list_by_state(&self, state: AgentState) -> Vec<Agent> {
        let agents = self.agents.read().await;
        agents
            .values()
            .filter(|a| a.state == state)
            .cloned()
            .collect()
    }

    /// Update agent state
    pub async fn update_state(&self, id: &str, new_state: AgentState) -> RegistryResult<Agent> {
        let mut agents = self.agents.write().await;
        let agent = agents
            .get_mut(id)
            .ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))?;

        // Validate state transition
        if !is_valid_transition(agent.state, new_state) {
            return Err(RegistryError::InvalidStateTransition(format!(
                "{:?} -> {:?}",
                agent.state, new_state
            )));
        }

        debug!(agent_id = %id, from = ?agent.state, to = ?new_state, "state transition");
        agent.set_state(new_state);
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

    /// Remove an agent (cleanup process and metadata)
    pub async fn remove(&self, id: &str) -> RegistryResult<Agent> {
        // Kill the process if running
        self.kill(id).await.ok();

        // Remove from processes map
        {
            let mut processes = self.processes.write().await;
            processes.remove(id);
        }

        // Remove from agents map
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

        // Update state to Stopped
        if let Ok(_agent) = self.update_state(id, AgentState::Stopped).await {
            debug!(agent_id = %id, "agent stopped");
        }

        Ok(())
    }

    /// Send a message to an agent via stdin
    pub async fn send_message(&self, id: &str, message: &str) -> RegistryResult<()> {
        let process = {
            let processes = self.processes.read().await;
            processes.get(id).cloned()
        };

        let mut process = process
            .ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))?;

        process.send_message(message).await?;
        Ok(())
    }

    /// Check for dead agents and clean them up
    pub async fn cleanup_dead(&self) -> Vec<String> {
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

        // Remove dead agents
        for id in &dead_ids {
            if let Err(e) = self.remove(id).await {
                error!(agent_id = %id, error = %e, "failed to cleanup dead agent");
            }
        }

        dead_ids
    }

    /// Get count of registered agents
    pub async fn count(&self) -> usize {
        let agents = self.agents.read().await;
        agents.len()
    }
}

/// Check if a state transition is valid
fn is_valid_transition(from: AgentState, to: AgentState) -> bool {
    use AgentState::*;
    match (from, to) {
        // Idle can transition to Running or Error
        (Idle, Running) | (Idle, Error) => true,
        // Running can transition to Waiting, Suspended, Stopped, or Error
        (Running, Waiting) | (Running, Suspended) | (Running, Stopped) | (Running, Error) => true,
        // Waiting can transition back to Running, Suspended, Stopped, or Error
        (Waiting, Running) | (Waiting, Suspended) | (Waiting, Stopped) | (Waiting, Error) => true,
        // Suspended can transition to Running or Stopped
        (Suspended, Running) | (Suspended, Stopped) => true,
        // Stopped and Error are terminal states
        (Stopped, _) | (Error, _) => false,
        // Same state is always valid (no-op)
        _ if from == to => true,
        // Everything else is invalid
        _ => false,
    }
}

/// Wrap AgentRegistry in Arc for shared access
pub type SharedAgentRegistry = Arc<AgentRegistry>;

/// Create a new shared agent registry
pub fn create_registry(heartbeat_timeout_secs: i64) -> SharedAgentRegistry {
    Arc::new(AgentRegistry::new(heartbeat_timeout_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_agent() {
        let registry = create_registry(30);
        let agent = registry.register("test-agent".to_string(), None).await;
        assert_eq!(agent.name, "test-agent");
        assert_eq!(agent.state, AgentState::Idle);
    }

    #[tokio::test]
    async fn test_get_agent() {
        let registry = create_registry(30);
        let created = registry.register("test".to_string(), None).await;
        let retrieved = registry.get(&created.id).await.unwrap();
        assert_eq!(retrieved.id, created.id);
    }

    #[tokio::test]
    async fn test_get_agent_not_found() {
        let registry = create_registry(30);
        let result = registry.get("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_agents() {
        let registry = create_registry(30);
        registry.register("agent1".to_string(), None).await;
        registry.register("agent2".to_string(), None).await;
        let agents = registry.list().await;
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_update_state() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await;
        registry.update_state(&agent.id, AgentState::Running).await.unwrap();
        let updated = registry.get(&agent.id).await.unwrap();
        assert_eq!(updated.state, AgentState::Running);
    }

    #[tokio::test]
    async fn test_invalid_state_transition() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await;
        // Can't go from Idle to Suspended directly
        let result = registry.update_state(&agent.id, AgentState::Suspended).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_remove_agent() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await;
        let removed = registry.remove(&agent.id).await.unwrap();
        assert_eq!(removed.id, agent.id);
        assert!(registry.get(&agent.id).await.is_err());
    }

    #[test]
    fn test_is_valid_transition() {
        assert!(is_valid_transition(AgentState::Idle, AgentState::Running));
        assert!(is_valid_transition(AgentState::Running, AgentState::Waiting));
        assert!(is_valid_transition(AgentState::Running, AgentState::Stopped));
        assert!(!is_valid_transition(AgentState::Stopped, AgentState::Running));
        assert!(!is_valid_transition(AgentState::Error, AgentState::Running));
    }
}
