//! Agent module - pure configuration layer for agent definitions.

pub mod config;
pub mod registry;
pub mod spawn;

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
    /// Parent agent ID (if this agent was spawned by another)
    pub parent_id: Option<String>,
    /// When this agent was created
    pub created_at: DateTime<Utc>,
}

impl Agent {
    /// Create a new agent config record
    /// Silently converts empty string parent_id to None (data corruption guard).
    pub fn new(name: String, parent_id: Option<String>) -> Self {
        let parent_id = parent_id.filter(|id| !id.is_empty());
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            parent_id,
            created_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_creation() {
        let agent = Agent::new("test-agent".to_string(), None);
        assert!(agent.parent_id.is_none());
        assert!(!agent.id.is_empty());
    }

    #[test]
    fn test_agent_with_parent() {
        let parent_id = "parent-123".to_string();
        let agent = Agent::new("child-agent".to_string(), Some(parent_id.clone()));
        assert_eq!(agent.parent_id, Some(parent_id));
    }
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod spawn_tests;
