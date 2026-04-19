//! Cascade lifecycle operations for AgentRegistry

use super::{
    Agent, AgentRegistry, AgentState, ErrorInfo, RegistryError, RegistryResult, SuspendedReason,
    TransitionTrigger,
};
use crate::agent::state::{Checkpoint, DestroyConfirmation, SourceLocation};
use tracing::debug;
use uuid::Uuid;

impl AgentRegistry {
    /// Stop this agent and all its descendants (children first, depth-first).
    pub async fn stop_with_descendants(&self, id: &str) -> RegistryResult<()> {
        let descendants = self.get_descendants(id).await;
        debug!(
            agent_id = %id,
            descendant_count = %descendants.len(),
            "stopping agent and descendants"
        );
        for descendant in descendants {
            self.update_state(
                &descendant.id,
                AgentState::Stopped,
                TransitionTrigger::ParentCascade,
            )
            .await
            .ok();
            self.kill(&descendant.id).await.ok();
        }
        self.update_state(id, AgentState::Stopped, TransitionTrigger::ParentCascade)
            .await?;
        self.kill(id).await?;
        Ok(())
    }

    /// Suspend this agent and all its descendants.
    pub async fn suspend_with_descendants(
        &self,
        id: &str,
        reason: SuspendedReason,
    ) -> RegistryResult<()> {
        let descendants = self.get_descendants(id).await;
        for descendant in descendants {
            if matches!(reason, SuspendedReason::Forced) {
                self.save_checkpoint(&descendant.id, "cascade-suspend")
                    .await
                    .ok();
            }
            self.update_state(
                &descendant.id,
                AgentState::Suspended(reason),
                TransitionTrigger::ParentCascade,
            )
            .await
            .ok();
        }
        if matches!(reason, SuspendedReason::Forced) {
            self.save_checkpoint(id, "cascade-suspend").await.ok();
        }
        self.update_state(
            id,
            AgentState::Suspended(reason),
            TransitionTrigger::ParentCascade,
        )
        .await?;
        Ok(())
    }
}

impl AgentRegistry {
    /// Resume an agent from suspended or error state.
    pub async fn resume(&self, id: &str) -> RegistryResult<Agent> {
        let agent = self.get(id).await?;
        match &agent.state {
            AgentState::Suspended(_)
            | AgentState::Error(ErrorInfo {
                recoverable: true, ..
            }) => {}
            AgentState::Error(ErrorInfo {
                recoverable: false, ..
            }) => {
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
            location: SourceLocation::new(location_note, file!(), line!()),
            variables_json: "{}".to_string(),
            parent_id: agent.parent_id.clone(),
            created_at: chrono::Utc::now(),
        };
        crate::agent::state::save_checkpoint(&checkpoint)
            .map_err(|e| RegistryError::ProcessError(e.into()))?;
        Ok(checkpoint)
    }
}

impl AgentRegistry {
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

    /// Suspend an agent with a reason, optionally cascading.
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
            self.update_state(
                id,
                AgentState::Suspended(reason),
                TransitionTrigger::Scheduler,
            )
            .await?;
        }
        Ok(())
    }

    /// Resume an agent from suspended or error state.
    pub async fn resume_agent(&self, id: &str) -> RegistryResult<Agent> {
        self.resume(id).await
    }

    /// Destroy an agent irreversibly.
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
                    agent.name, agent.state
                ),
                confirm_token,
            }));
        }
        self.stop_with_descendants(id).await?;
        self.remove(id).await?;
        Ok(None)
    }

    /// Confirm a destroy operation using the token returned by `destroy_agent`.
    pub async fn confirm_destroy(&self, id: &str, confirm_token: &str) -> RegistryResult<()> {
        if confirm_token.is_empty() {
            return Err(RegistryError::DestroyConfirmationRequired);
        }
        self.stop_with_descendants(id).await?;
        self.remove(id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::registry::create_registry;

    #[tokio::test]
    async fn test_resume_from_suspended() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        registry
            .update_state(
                &agent.id,
                AgentState::Suspended(SuspendedReason::SelfRequested),
                TransitionTrigger::Scheduler,
            )
            .await
            .unwrap();
        let resumed = registry.resume(&agent.id).await.unwrap();
        assert!(matches!(resumed.state, AgentState::Running));
    }

    #[tokio::test]
    async fn test_resume_from_recoverable_error() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        registry
            .update_state(
                &agent.id,
                AgentState::Error(ErrorInfo::new("oops", true)),
                TransitionTrigger::Error,
            )
            .await
            .unwrap();
        let resumed = registry.resume(&agent.id).await.unwrap();
        assert!(matches!(resumed.state, AgentState::Running));
    }

    #[tokio::test]
    async fn test_resume_from_non_recoverable_error_fails() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
        registry
            .update_state(
                &agent.id,
                AgentState::Error(ErrorInfo::new("fatal", false)),
                TransitionTrigger::Error,
            )
            .await
            .unwrap();
        let result = registry.resume(&agent.id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_destroy_agent_requires_confirmation() {
        let registry = create_registry(30);
        let agent = registry.register("test".to_string(), None).await.unwrap();
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
        let result = registry.destroy_agent(&agent.id, false).await.unwrap();
        assert!(result.is_none());
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
            .update_state(
                &parent.id,
                AgentState::Running,
                TransitionTrigger::Scheduler,
            )
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
            .update_state(
                &parent.id,
                AgentState::Running,
                TransitionTrigger::Scheduler,
            )
            .await
            .unwrap();
        registry
            .update_state(&child.id, AgentState::Running, TransitionTrigger::Scheduler)
            .await
            .unwrap();
        registry
            .suspend_agent(&parent.id, SuspendedReason::Forced, true)
            .await
            .unwrap();
        let parent_state = registry.get(&parent.id).await.unwrap().state;
        let child_state = registry.get(&child.id).await.unwrap().state;
        assert!(matches!(
            parent_state,
            AgentState::Suspended(SuspendedReason::Forced)
        ));
        assert!(matches!(
            child_state,
            AgentState::Suspended(SuspendedReason::Forced)
        ));
    }
}
