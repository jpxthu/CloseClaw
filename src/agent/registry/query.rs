//! Query methods for AgentRegistry

use super::{Agent, AgentRegistry, AgentState, RegistryError, RegistryResult};
use tracing::warn;

impl AgentRegistry {
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
}

impl AgentRegistry {
    /// Get direct children of an agent
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
        let current = match agents.get(agent_id) {
            Some(a) => a,
            None => return ancestors,
        };
        let mut current_parent_id = current.parent_id.clone();
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

    /// Check if agent_a is an ancestor of agent_b
    pub async fn is_ancestor_of(&self, ancestor_id: &str, descendant_id: &str) -> bool {
        let ancestors = self.get_ancestors(descendant_id).await;
        ancestors.iter().any(|a| a.id == ancestor_id)
    }

    /// Get all descendants of an agent (recursive children, breadth-first).
    pub async fn get_descendants(&self, agent_id: &str) -> Vec<Agent> {
        use std::collections::VecDeque;
        let mut descendants = Vec::new();
        let mut queue: VecDeque<String> = VecDeque::new();
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
}

impl AgentRegistry {
    /// Get count of registered agents
    pub async fn count(&self) -> usize {
        let agents = self.agents.read().await;
        agents.len()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::registry::create_registry;

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
    async fn test_get_children() {
        let registry = create_registry(30);
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
        let children = registry.get_children(&parent.id).await;
        assert_eq!(children.len(), 2);
        let child_ids: Vec<_> = children.iter().map(|a| a.name.clone()).collect();
        assert!(child_ids.contains(&"child1".to_string()));
        assert!(child_ids.contains(&"child2".to_string()));
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
        let found_parent = registry.get_parent(&child.id).await;
        assert!(found_parent.is_some());
        assert_eq!(found_parent.unwrap().id, parent.id);
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
        let ancestors = registry.get_ancestors(&grandchild.id).await;
        assert_eq!(ancestors.len(), 2);
        assert_eq!(ancestors[0].name, "child");
        assert_eq!(ancestors[1].name, "root");
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
        assert!(registry.is_ancestor_of(&root.id, &grandchild.id).await);
        assert!(registry.is_ancestor_of(&child.id, &grandchild.id).await);
        assert!(!registry.is_ancestor_of(&grandchild.id, &root.id).await);
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
        let child1_descendants = registry.get_descendants(&child1.id).await;
        assert_eq!(child1_descendants.len(), 1);
        assert_eq!(child1_descendants[0].name, "grandchild");
        let grandchild_descendants = registry.get_descendants(&grandchild.id).await;
        assert!(grandchild_descendants.is_empty());
    }
}
