//! Agent Runtime - manages agent lifecycle and inter-agent communication

pub mod config;
pub mod process;
pub mod registry;
pub mod state;

pub use state::{
    is_valid_transition, AgentState, AgentStateTransition, Checkpoint, DestroyConfirmation,
    ErrorInfo, PausePoint, SourceLocation, SuspendedReason, TransitionTrigger,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Agent instance - represents a single agent with its metadata
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Agent {
    /// Unique identifier for this agent
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Current lifecycle state
    pub state: AgentState,
    /// Parent agent ID (if this agent was spawned by another)
    pub parent_id: Option<String>,
    /// When this agent was created
    pub created_at: DateTime<Utc>,
    /// Last heartbeat received from this agent
    pub last_heartbeat: DateTime<Utc>,
}

impl Agent {
    /// Create a new agent in Idle state
    /// Silently converts empty string parent_id to None (data corruption guard).
    pub fn new(name: String, parent_id: Option<String>) -> Self {
        let parent_id = parent_id.filter(|id| !id.is_empty());
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            state: AgentState::Idle,
            parent_id,
            created_at: now,
            last_heartbeat: now,
        }
    }

    /// Update the agent's state
    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
    }

    /// Update the last heartbeat timestamp
    pub fn update_heartbeat(&mut self) {
        self.last_heartbeat = Utc::now();
    }

    /// Check if the agent is still alive (heartbeat within threshold)
    pub fn is_alive(&self, heartbeat_timeout_secs: i64) -> bool {
        let elapsed = Utc::now()
            .signed_duration_since(self.last_heartbeat)
            .num_seconds();
        elapsed < heartbeat_timeout_secs
    }

    /// Check if the agent is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, AgentState::Stopped | AgentState::Error(_))
    }

    /// Emit a state transition event for the given state change.
    pub fn emit_transition(
        &self,
        from_state: AgentState,
        to_state: AgentState,
        trigger: TransitionTrigger,
    ) -> AgentStateTransition {
        AgentStateTransition::new(from_state, to_state, trigger)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_creation() {
        let agent = Agent::new("test-agent".to_string(), None);
        assert!(matches!(agent.state, AgentState::Idle));
        assert!(agent.parent_id.is_none());
        assert!(!agent.id.is_empty());
    }

    #[test]
    fn test_agent_with_parent() {
        let parent_id = "parent-123".to_string();
        let agent = Agent::new("child-agent".to_string(), Some(parent_id.clone()));
        assert_eq!(agent.parent_id, Some(parent_id));
    }

    #[test]
    fn test_is_alive() {
        let agent = Agent::new("test".to_string(), None);
        // Fresh agent should be alive with a reasonable timeout
        assert!(agent.is_alive(60));
        // Should not be alive with zero timeout
        assert!(!agent.is_alive(0));
    }

    #[test]
    fn test_is_terminal() {
        let mut agent = Agent::new("test".to_string(), None);
        assert!(!agent.is_terminal());

        agent.set_state(AgentState::Stopped);
        assert!(agent.is_terminal());

        agent.set_state(AgentState::Error(ErrorInfo::new("oops", false)));
        assert!(agent.is_terminal());

        agent.set_state(AgentState::Running);
        assert!(!agent.is_terminal());

        agent.set_state(AgentState::Suspended(SuspendedReason::Forced));
        assert!(!agent.is_terminal());
    }

    #[test]
    fn test_emit_transition() {
        let agent = Agent::new("test".to_string(), None);
        let t = agent.emit_transition(
            AgentState::Idle,
            AgentState::Running,
            TransitionTrigger::UserRequest,
        );
        assert!(matches!(t.from_state, AgentState::Idle));
        assert!(matches!(t.to_state, AgentState::Running));
        assert!(matches!(t.trigger, TransitionTrigger::UserRequest));
    }
}
