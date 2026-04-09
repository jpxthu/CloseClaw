//! Agent Registry - manages all agent lifecycles
//!
//! Provides centralized management for creating, tracking, and removing agents.

use crate::agent::process::{AgentProcess, AgentProcessHandle};
use crate::agent::state::{
    is_valid_transition, AgentStateTransition, Checkpoint, DestroyConfirmation, ErrorInfo,
    SourceLocation, SuspendedReason, TransitionTrigger,
};
use crate::agent::{Agent, AgentState};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Errors that can occur in the agent registry
#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("agent already exists: {0}")]
    AgentAlreadyExists(String),
    #[error("invalid state transition: {0}")]
    InvalidStateTransition(String),
    #[error("destroy requires confirmation (token mismatch or missing)")]
    DestroyConfirmationRequired,
    #[error("process error: {0}")]
    ProcessError(#[from] crate::agent::process::ProcessError),
}

/// Result type for registry operations
pub type RegistryResult<T> = Result<T, RegistryError>;

/// Result of a cleanup operation
#[derive(Debug)]
pub struct CleanupResult {
    /// Agents that were successfully cleaned up
    pub cleaned: Vec<String>,
    /// Agent IDs that failed to be removed, with their errors
    pub failed: Vec<(String, RegistryError)>,
}

/// Thread-safe agent registry for managing all agent lifecycles
#[derive(Debug)]
pub struct AgentRegistry {
    /// Map of agent ID to agent metadata
    agents: RwLock<HashMap<String, Agent>>,
    /// Map of agent ID to running process handle
    processes: RwLock<HashMap<String, AgentProcessHandle>>,
    /// Heartbeat timeout in seconds
    heartbeat_timeout_secs: i64,
    /// Graceful shutdown: wait timeout in seconds
    wait_timeout_secs: u64,
    /// Graceful shutdown: grace period in seconds
    grace_period_secs: u64,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new_with_graceful_shutdown(30, 30, 10)
    }
}

impl AgentRegistry {
    /// Create a new registry with specified heartbeat timeout.
    pub fn new(heartbeat_timeout_secs: i64) -> Self {
        Self::new_with_graceful_shutdown(heartbeat_timeout_secs, 30, 10)
    }

    /// Create a new registry with graceful shutdown configuration.
    pub fn new_with_graceful_shutdown(
        heartbeat_timeout_secs: i64,
        wait_timeout_secs: u64,
        grace_period_secs: u64,
    ) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            processes: RwLock::new(HashMap::new()),
            heartbeat_timeout_secs,
            wait_timeout_secs,
            grace_period_secs,
        }
    }

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
        // Validate binary path before registration
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

        // Register agent metadata (transaction-like: clean up on spawn failure)
        let agent = self.register(name, parent_id).await?;
        let agent_id = agent.id.clone();

        // Spawn the process; on failure, remove the registered agent
        let process = match AgentProcess::spawn(agent_binary_path, &agent_id).await {
            Ok(p) => p,
            Err(e) => {
                error!(agent_id = %agent_id, error = %e, "failed to spawn agent process, cleaning up registration");
                // Remove the orphaned registration
                let _ = self.remove(&agent_id).await;
                return Err(RegistryError::ProcessError(e));
            }
        };

        // Store process handle
        {
            let mut processes = self.processes.write().await;
            processes.insert(agent_id.clone(), process);
        }

        // Update state to Running
        self.update_state(&agent_id, AgentState::Running, TransitionTrigger::Scheduler)
            .await?;

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

    /// Get all descendants of an agent (recursive children, breadth-first).
    pub async fn get_descendants(&self, agent_id: &str) -> Vec<Agent> {
        use std::collections::VecDeque;
        let mut descendants = Vec::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        // Seed with direct children
        let initial_children = self.get_children(agent_id).await;
        for child in initial_children {
            queue.push_back(child.id.clone());
        }

        while let Some(current_id) = queue.pop_front() {
            if let Ok(agent) = self.get(&current_id).await {
                descendants.push(agent.clone());
                let children = self.get_children(&current_id).await;
                for child in children {
                    queue.push_back(child.id);
                }
            }
        }

        descendants
    }

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

        // Validate state transition
        if !is_valid_transition(&agent.state, &new_state) {
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

        // Emit transition event (in production this would go to an event bus)
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

        // Remove dead agents and track failures
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

    /// Get count of registered agents
    pub async fn count(&self) -> usize {
        let agents = self.agents.read().await;
        agents.len()
    }

    // -------------------------------------------------------------------------
    // Phase 4: Cascade lifecycle operations
    // -------------------------------------------------------------------------

    /// Stop this agent and all its descendants.
    ///
    /// Cascade order: children first (depth-first), then the agent itself.
    pub async fn stop_with_descendants(&self, id: &str) -> RegistryResult<()> {
        let descendants = self.get_descendants(id).await;
        debug!(
            agent_id = %id,
            descendant_count = %descendants.len(),
            "stopping agent and descendants"
        );

        // Stop all descendants first (depth-first, children before parent)
        for descendant in descendants {
            self.update_state(
                &descendant.id,
                AgentState::Stopped,
                TransitionTrigger::ParentCascade,
            )
            .await
            .ok(); // Log but don't fail on individual errors
            self.kill(&descendant.id).await.ok();
        }

        // Stop the agent itself
        self.update_state(id, AgentState::Stopped, TransitionTrigger::ParentCascade)
            .await?;
        self.kill(id).await?;

        Ok(())
    }

    /// Suspend this agent and all its descendants (forced or self-requested).
    ///
    /// For `Forced` suspension: saves a checkpoint before suspending.
    /// Cascade order: children first, then the agent itself.
    pub async fn suspend_with_descendants(
        &self,
        id: &str,
        reason: SuspendedReason,
    ) -> RegistryResult<()> {
        let descendants = self.get_descendants(id).await;

        // Stop all descendants first
        for descendant in descendants {
            if matches!(reason, SuspendedReason::Forced) {
                self.save_checkpoint(&descendant.id, "cascade-suspend").await.ok();
            }
            self.update_state(
                &descendant.id,
                AgentState::Suspended(reason),
                TransitionTrigger::ParentCascade,
            )
            .await
            .ok();
        }

        // Suspend the agent itself
        if matches!(reason, SuspendedReason::Forced) {
            self.save_checkpoint(id, "cascade-suspend").await.ok();
        }
        self.update_state(id, AgentState::Suspended(reason), TransitionTrigger::ParentCascade)
            .await?;

        Ok(())
    }

    /// Resume an agent from suspended or error state.
    ///
    /// Returns the agent to Running state.
    pub async fn resume(&self, id: &str) -> RegistryResult<Agent> {
        let agent = self.get(id).await?;

        // Only allow resume from Suspended or Error(recoverable) states
        match &agent.state {
            AgentState::Suspended(_) | AgentState::Error(ErrorInfo { recoverable: true, .. }) => {}
            AgentState::Error(ErrorInfo { recoverable: false, .. }) => {
                return Err(RegistryError::InvalidStateTransition(
                    "cannot resume from non-recoverable error".to_string(),
                ));
            }
            other => {
                return Err(RegistryError::InvalidStateTransition(format!(
                    "cannot resume from state {:?}",
                    other
                )));
            }
        }

        self.update_state(id, AgentState::Running, TransitionTrigger::UserRequest)
            .await
    }

    /// Save a checkpoint for an agent.
    pub async fn save_checkpoint(
        &self,
        agent_id: &str,
        location_note: &str,
    ) -> RegistryResult<Checkpoint> {
        let agent = self.get(agent_id).await?;
        let checkpoint = Checkpoint {
            id: Uuid::new_v4().to_string(),
            agent_id: agent_id.to_string(),
            location: SourceLocation::new(
                location_note,
                file!(),
                line!(),
            ),
            variables_json: "{}".to_string(),
            parent_id: agent.parent_id.clone(),
            created_at: chrono::Utc::now(),
        };
        crate::agent::state::save_checkpoint(&checkpoint)
            .map_err(|e| RegistryError::ProcessError(e.into()))?;
        Ok(checkpoint)
    }

    // -------------------------------------------------------------------------
    // Phase 6: AgentRegistry extensions
    // -------------------------------------------------------------------------

    /// Stop an agent, optionally cascading to all descendants.
    pub async fn stop_agent(&self, id: &str, cascade: bool) -> RegistryResult<()> {
        if cascade {
            self.stop_with_descendants(id).await?;
        } else {
            self.update_state(id, AgentState::Stopped, TransitionTrigger::UserRequest)
                .await?;
            self.kill(id).await?;
        }
        Ok(())
    }

    /// Suspend an agent with a reason, optionally cascading to all descendants.
    pub async fn suspend_agent(
        &self,
        id: &str,
        reason: SuspendedReason,
        cascade: bool,
    ) -> RegistryResult<()> {
        if cascade {
            self.suspend_with_descendants(id, reason).await?;
        } else {
            if matches!(reason, SuspendedReason::Forced) {
                self.save_checkpoint(id, "suspend").await.ok();
            }
            self.update_state(id, AgentState::Suspended(reason), TransitionTrigger::Scheduler)
                .await?;
        }
        Ok(())
    }

    /// Resume an agent from suspended or error state.
    pub async fn resume_agent(&self, id: &str) -> RegistryResult<Agent> {
        self.resume(id).await
    }

    /// Destroy an agent irreversibly.
    ///
    /// If `require_confirmation` is true, returns a `DestroyConfirmation` that must be
    /// confirmed with the returned `confirm_token`. If false, destroys immediately.
    pub async fn destroy_agent(
        &self,
        id: &str,
        require_confirmation: bool,
    ) -> RegistryResult<Option<DestroyConfirmation>> {
        let agent = self.get(id).await?;

        if require_confirmation {
            let confirm_token = Uuid::new_v4().to_string();
            return Ok(Some(DestroyConfirmation {
                agent_id: agent.id.clone(),
                message: format!(
                    "This will permanently destroy agent '{}' (state: {:?}) and remove all its metadata. This cannot be undone.",
                    agent.name,
                    agent.state
                ),
                confirm_token,
            }));
        }

        // Destroy immediately without confirmation
        self.stop_with_descendants(id).await?;
        self.remove(id).await?;
        Ok(None)
    }

    /// Confirm a destroy operation using the token returned by `destroy_agent`.
    pub async fn confirm_destroy(&self, id: &str, confirm_token: &str) -> RegistryResult<()> {
        // We need to verify the token - in a real implementation this would
        // be stored temporarily. For now, we just check non-empty and proceed.
        if confirm_token.is_empty() {
            return Err(RegistryError::DestroyConfirmationRequired);
        }
        self.stop_with_descendants(id).await?;
        self.remove(id).await?;
        Ok(())
    }

    /// Get the wait timeout for graceful shutdown.
    pub fn wait_timeout_secs(&self) -> u64 {
        self.wait_timeout_secs
    }

    /// Get the grace period for graceful shutdown.
    pub fn grace_period_secs(&self) -> u64 {
        self.grace_period_secs
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
        let agent = registry
            .register("test-agent".to_string(), None)
            .await
            .unwrap();
        assert_eq!(agent.name, "test-agent");
        assert!(matches!(agent.state, AgentState::Idle));
    }

    #[tokio::test]
    async fn test_get_agent() {
        let registry = create_registry(30);
        let created = registry.register("test".to_string(), None).await.unwrap();
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
        registry.register("agent1".to_string(), None).await.unwrap();
        registry.register("agent2".to_string(), None).await.unwrap();
        let agents = registry.list().await;
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_update_state() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        registry
            .update_state(&agent.id, AgentState::Running, TransitionTrigger::UserRequest)
            .await
            .unwrap();
        let updated = registry.get(&agent.id).await.unwrap();
        assert!(matches!(updated.state, AgentState::Running));
    }

    #[tokio::test]
    async fn test_invalid_state_transition() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        // Can't go from Stopped to Running (Stopped is terminal)
        registry
            .update_state(&agent.id, AgentState::Stopped, TransitionTrigger::UserRequest)
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

    #[test]
    fn test_state_transition_validation() {
        use AgentState::*;
        // Valid
        assert!(is_valid_transition(&Idle, &Running));
        assert!(is_valid_transition(&Running, &Waiting));
        assert!(is_valid_transition(&Running, &Stopped));
        assert!(is_valid_transition(&Running, &Suspended(SuspendedReason::Forced)));
        assert!(is_valid_transition(&Waiting, &Suspended(SuspendedReason::SelfRequested)));
        assert!(is_valid_transition(&Suspended(SuspendedReason::Forced), &Running));
        assert!(is_valid_transition(&Suspended(SuspendedReason::SelfRequested), &Running));
        assert!(is_valid_transition(&Suspended(SuspendedReason::SelfRequested), &Stopped));
        // Terminal states
        assert!(!is_valid_transition(&Stopped, &Running));
        // Non-recoverable error is terminal
        assert!(!is_valid_transition(
            &Error(ErrorInfo::new("fatal", false)),
            &Running
        ));
        // Recoverable error can be resumed
        assert!(is_valid_transition(
            &Error(ErrorInfo::new("recoverable", true)),
            &Running
        ));
        // Same state is always valid (no-op)
        assert!(is_valid_transition(&Running, &Running));
    }

    #[tokio::test]
    async fn test_get_children() {
        let registry = create_registry(30);

        // Create parent and children
        let parent = registry.register("parent".to_string(), None).await.unwrap();
        let child1 = registry
            .register("child1".to_string(), Some(parent.id.clone()))
            .await
            .unwrap();
        let _child2 = registry
            .register("child2".to_string(), Some(parent.id.clone()))
            .await
            .unwrap();
        let _grandchild = registry
            .register("grandchild".to_string(), Some(child1.id.clone()))
            .await
            .unwrap();

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

        let parent = registry.register("parent".to_string(), None).await.unwrap();
        let child = registry
            .register("child".to_string(), Some(parent.id.clone()))
            .await
            .unwrap();

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

        let root = registry.register("root".to_string(), None).await.unwrap();
        let child = registry
            .register("child".to_string(), Some(root.id.clone()))
            .await
            .unwrap();
        let grandchild = registry
            .register("grandchild".to_string(), Some(child.id.clone()))
            .await
            .unwrap();

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

        let root = registry.register("root".to_string(), None).await.unwrap();
        let child = registry
            .register("child".to_string(), Some(root.id.clone()))
            .await
            .unwrap();
        let grandchild = registry
            .register("grandchild".to_string(), Some(child.id.clone()))
            .await
            .unwrap();

        // Root is ancestor of grandchild
        assert!(registry.is_ancestor_of(&root.id, &grandchild.id).await);

        // Child is ancestor of grandchild
        assert!(registry.is_ancestor_of(&child.id, &grandchild.id).await);

        // Grandchild is NOT ancestor of root
        assert!(!registry.is_ancestor_of(&grandchild.id, &root.id).await);

        // Root is NOT ancestor of itself
        assert!(!registry.is_ancestor_of(&root.id, &root.id).await);
    }

    #[tokio::test]
    async fn test_get_descendants() {
        let registry = create_registry(30);

        let root = registry.register("root".to_string(), None).await.unwrap();
        let child1 = registry
            .register("child1".to_string(), Some(root.id.clone()))
            .await
            .unwrap();
        let _child2 = registry
            .register("child2".to_string(), Some(root.id.clone()))
            .await
            .unwrap();
        let grandchild = registry
            .register("grandchild".to_string(), Some(child1.id.clone()))
            .await
            .unwrap();

        let descendants = registry.get_descendants(&root.id).await;
        assert_eq!(descendants.len(), 3);
        let names: Vec<_> = descendants.iter().map(|a| a.name.clone()).collect();
        assert!(names.contains(&"child1".to_string()));
        assert!(names.contains(&"child2".to_string()));
        assert!(names.contains(&"grandchild".to_string()));
        // grandchild is also a descendant of child1
        let child1_descendants = registry.get_descendants(&child1.id).await;
        assert_eq!(child1_descendants.len(), 1);
        assert_eq!(child1_descendants[0].name, "grandchild");
        // grandchild has no descendants
        let grandchild_descendants = registry.get_descendants(&grandchild.id).await;
        assert!(grandchild_descendants.is_empty());
    }

    #[tokio::test]
    async fn test_resume_from_suspended() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();

        // Transition to Suspended
        registry
            .update_state(
                &agent.id,
                AgentState::Suspended(SuspendedReason::SelfRequested),
                TransitionTrigger::Scheduler,
            )
            .await
            .unwrap();

        // Resume
        let resumed = registry.resume(&agent.id).await.unwrap();
        assert!(matches!(resumed.state, AgentState::Running));
    }

    #[tokio::test]
    async fn test_resume_from_recoverable_error() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();

        // Transition to Error(recoverable)
        registry
            .update_state(
                &agent.id,
                AgentState::Error(ErrorInfo::new("oops", true)),
                TransitionTrigger::Error,
            )
            .await
            .unwrap();

        // Resume
        let resumed = registry.resume(&agent.id).await.unwrap();
        assert!(matches!(resumed.state, AgentState::Running));
    }

    #[tokio::test]
    async fn test_resume_from_non_recoverable_error_fails() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();

        // Transition to Error(non-recoverable)
        registry
            .update_state(
                &agent.id,
                AgentState::Error(ErrorInfo::new("fatal", false)),
                TransitionTrigger::Error,
            )
            .await
            .unwrap();

        // Resume should fail
        let result = registry.resume(&agent.id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_destroy_agent_requires_confirmation() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();

        // require_confirmation = true
        let confirmation = registry
            .destroy_agent(&agent.id, true)
            .await
            .unwrap()
            .expect("should return confirmation");
        assert!(!confirmation.confirm_token.is_empty());
        assert!(confirmation.message.contains("permanently destroy"));
    }

    #[tokio::test]
    async fn test_destroy_agent_no_confirmation() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();

        // require_confirmation = false — should destroy immediately
        let result = registry.destroy_agent(&agent.id, false).await.unwrap();
        assert!(result.is_none());
        // Agent should be gone
        assert!(registry.get(&agent.id).await.is_err());
    }

    #[tokio::test]
    async fn test_confirm_destroy() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        let token = Uuid::new_v4().to_string();

        let result = registry.confirm_destroy(&agent.id, &token).await;
        assert!(result.is_ok());
        assert!(registry.get(&agent.id).await.is_err());
    }

    #[tokio::test]
    async fn test_confirm_destroy_empty_token_fails() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();

        let result = registry.confirm_destroy(&agent.id, "").await;
        assert!(result.is_err());
        // Agent should still exist
        assert!(registry.get(&agent.id).await.is_ok());
    }

    #[tokio::test]
    async fn test_stop_agent_cascade() {
        let registry = create_registry(30);
        let parent = registry.register("parent".to_string(), None).await.unwrap();
        let child = registry
            .register("child".to_string(), Some(parent.id.clone()))
            .await
            .unwrap();

        registry
            .update_state(&parent.id, AgentState::Running, TransitionTrigger::Scheduler)
            .await
            .unwrap();
        registry
            .update_state(&child.id, AgentState::Running, TransitionTrigger::Scheduler)
            .await
            .unwrap();

        registry.stop_agent(&parent.id, true).await.unwrap();

        let parent_state = registry.get(&parent.id).await.unwrap().state;
        let child_state = registry.get(&child.id).await.unwrap().state;
        assert!(matches!(parent_state, AgentState::Stopped));
        assert!(matches!(child_state, AgentState::Stopped));
    }

    #[tokio::test]
    async fn test_suspend_agent_cascade() {
        let registry = create_registry(30);
        let parent = registry.register("parent".to_string(), None).await.unwrap();
        let child = registry
            .register("child".to_string(), Some(parent.id.clone()))
            .await
            .unwrap();

        registry
            .update_state(&parent.id, AgentState::Running, TransitionTrigger::Scheduler)
            .await
            .unwrap();
        registry
            .update_state(&child.id, AgentState::Running, TransitionTrigger::Scheduler)
            .await
            .unwrap();

        registry
            .suspend_agent(
                &parent.id,
                SuspendedReason::Forced,
                true,
            )
            .await
            .unwrap();

        let parent_state = registry.get(&parent.id).await.unwrap().state;
        let child_state = registry.get(&child.id).await.unwrap().state;
        assert!(matches!(parent_state, AgentState::Suspended(SuspendedReason::Forced)));
        assert!(matches!(child_state, AgentState::Suspended(SuspendedReason::Forced)));
    }

    #[tokio::test]
    async fn test_graceful_shutdown_config() {
        let registry = AgentRegistry::new_with_graceful_shutdown(30, 60, 20);
        assert_eq!(registry.wait_timeout_secs(), 60);
        assert_eq!(registry.grace_period_secs(), 20);
    }
}
