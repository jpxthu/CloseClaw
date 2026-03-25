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

    /// Get direct children of an agent (agents whose parent_id matches this agent's ID)
    pub async fn get_children(&self, parent_id: &str) -> Vec<Agent> {
        let agents = self.agents.read().await;
        agents
            .values()
            .filter(|a| a.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect()
    }

    /// Get the parent of an agent, if any
    pub async fn get_parent(&self, agent_id: &str) -> Option<Agent> {
        let agents = self.agents.read().await;
        agents
            .get(agent_id)
            .and_then(|a| a.parent_id.as_ref())
            .and_then(|pid| agents.get(pid).cloned())
    }

    /// Get the ancestor chain of an agent (excluding the agent itself)
    pub async fn get_ancestors(&self, agent_id: &str) -> Vec<Agent> {
        let agents = self.agents.read().await;
        let mut ancestors = Vec::new();

        // Get the starting agent to find its parent_id
        let current = match agents.get(agent_id) {
            Some(a) => a,
            None => return ancestors,
        };

        let mut current_parent_id = current.parent_id.clone();

        // Traverse up the hierarchy
        while let Some(parent_id) = current_parent_id {
            if parent_id.is_empty() {
                break;
            }
            match agents.get(&parent_id) {
                Some(parent) => {
                    ancestors.push(parent.clone());
                    current_parent_id = parent.parent_id.clone();
                }
                None => break,
            }
        }

        ancestors
    }

    /// Check if agent_a is an ancestor of agent_b (i.e., agent_b is a descendant of agent_a)
    pub async fn is_ancestor_of(&self, ancestor_id: &str, descendant_id: &str) -> bool {
        let ancestors = self.get_ancestors(descendant_id).await;
        ancestors.iter().any(|a| a.id == ancestor_id)
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

        let mut process = process.ok_or_else(|| RegistryError::AgentNotFound(id.to_string()))?;

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
        registry
            .update_state(&agent.id, AgentState::Running)
            .await
            .unwrap();
        let updated = registry.get(&agent.id).await.unwrap();
        assert_eq!(updated.state, AgentState::Running);
    }

    #[tokio::test]
    async fn test_invalid_state_transition() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await;
        // Can't go from Idle to Suspended directly
        let result = registry
            .update_state(&agent.id, AgentState::Suspended)
            .await;
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
        assert!(is_valid_transition(
            AgentState::Running,
            AgentState::Waiting
        ));
        assert!(is_valid_transition(
            AgentState::Running,
            AgentState::Stopped
        ));
        assert!(!is_valid_transition(
            AgentState::Stopped,
            AgentState::Running
        ));
        assert!(!is_valid_transition(AgentState::Error, AgentState::Running));
    }

    #[tokio::test]
    async fn test_get_children() {
        let registry = create_registry(30);

        // Create parent and children
        let parent = registry.register("parent".to_string(), None).await;
        let child1 = registry
            .register("child1".to_string(), Some(parent.id.clone()))
            .await;
        let _child2 = registry
            .register("child2".to_string(), Some(parent.id.clone()))
            .await;
        let _grandchild = registry
            .register("grandchild".to_string(), Some(child1.id.clone()))
            .await;

        // Get children of parent
        let children = registry.get_children(&parent.id).await;
        assert_eq!(children.len(), 2);
        let child_ids: Vec<_> = children.iter().map(|a| a.name.clone()).collect();
        assert!(child_ids.contains(&"child1".to_string()));
        assert!(child_ids.contains(&"child2".to_string()));

        // Get children of child1 (should include grandchild)
        let children_of_child1 = registry.get_children(&child1.id).await;
        assert_eq!(children_of_child1.len(), 1);
        assert_eq!(children_of_child1[0].name, "grandchild");
    }

    #[tokio::test]
    async fn test_get_parent() {
        let registry = create_registry(30);

        let parent = registry.register("parent".to_string(), None).await;
        let child = registry
            .register("child".to_string(), Some(parent.id.clone()))
            .await;

        // Get parent of child
        let found_parent = registry.get_parent(&child.id).await;
        assert!(found_parent.is_some());
        assert_eq!(found_parent.unwrap().id, parent.id);

        // Parent has no parent
        let parent_of_parent = registry.get_parent(&parent.id).await;
        assert!(parent_of_parent.is_none());
    }

    #[tokio::test]
    async fn test_get_ancestors() {
        let registry = create_registry(30);

        let root = registry.register("root".to_string(), None).await;
        let child = registry
            .register("child".to_string(), Some(root.id.clone()))
            .await;
        let grandchild = registry
            .register("grandchild".to_string(), Some(child.id.clone()))
            .await;

        // Get ancestors of grandchild
        let ancestors = registry.get_ancestors(&grandchild.id).await;
        assert_eq!(ancestors.len(), 2);
        assert_eq!(ancestors[0].name, "child");
        assert_eq!(ancestors[1].name, "root");

        // Root has no ancestors
        let root_ancestors = registry.get_ancestors(&root.id).await;
        assert!(root_ancestors.is_empty());
    }

    #[tokio::test]
    async fn test_is_ancestor_of() {
        let registry = create_registry(30);

        let root = registry.register("root".to_string(), None).await;
        let child = registry
            .register("child".to_string(), Some(root.id.clone()))
            .await;
        let grandchild = registry
            .register("grandchild".to_string(), Some(child.id.clone()))
            .await;

        // Root is ancestor of grandchild
        assert!(registry.is_ancestor_of(&root.id, &grandchild.id).await);

        // Child is ancestor of grandchild
        assert!(registry.is_ancestor_of(&child.id, &grandchild.id).await);

        // Grandchild is NOT ancestor of root
        assert!(!registry.is_ancestor_of(&grandchild.id, &root.id).await);

        // Root is NOT ancestor of itself
        assert!(!registry.is_ancestor_of(&root.id, &root.id).await);
    }
}
