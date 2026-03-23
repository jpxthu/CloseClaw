//! Agent Runtime - manages agent lifecycle and inter-agent communication

pub mod config;
pub mod process;
pub mod registry;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Agent lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Created, not started
    Idle,
    /// Actively processing
    Running,
    /// Waiting for response
    Waiting,
    /// Paused by scheduler
    Suspended,
    /// Completed or killed
    Stopped,
    /// Crashed with error
    Error,
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentState::Idle => write!(f, "idle"),
            AgentState::Running => write!(f, "running"),
            AgentState::Waiting => write!(f, "waiting"),
            AgentState::Suspended => write!(f, "suspended"),
            AgentState::Stopped => write!(f, "stopped"),
            AgentState::Error => write!(f, "error"),
        }
    }
}

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
    pub fn new(name: String, parent_id: Option<String>) -> Self {
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
        matches!(self.state, AgentState::Stopped | AgentState::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_creation() {
        let agent = Agent::new("test-agent".to_string(), None);
        assert_eq!(agent.state, AgentState::Idle);
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

        agent.set_state(AgentState::Error);
        assert!(agent.is_terminal());

        agent.set_state(AgentState::Running);
        assert!(!agent.is_terminal());
    }
}
