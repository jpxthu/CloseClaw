//! Tests for persistence module

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;

    use crate::session::persistence::{
        PendingMessage, PersistenceService, ReasoningMode, ReasoningModeState, SessionCheckpoint,
        SessionStatus,
    };
    use crate::session::storage::memory::MemoryStorage;
    use crate::session::CheckpointManager;

    fn create_test_checkpoint(session_id: &str) -> SessionCheckpoint {
        let mut state = ReasoningModeState::default();
        state.start_step(3);
        state.add_step_message("Step 1: Analyzing...".to_string());

        SessionCheckpoint::new(session_id.to_string())
            .with_last_message_id(Some("msg123".to_string()))
            .with_mode(ReasoningMode::Plan)
            .with_mode_state(state)
            .add_pending_message(PendingMessage::new(
                "pending1".to_string(),
                "Pending content".to_string(),
            ))
    }

    #[tokio::test]
    async fn test_checkpoint_manager_save_and_load() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let checkpoint = create_test_checkpoint("session1");

        // Save
        manager.save(checkpoint.clone()).await.unwrap();

        // Give the async task time to complete (for storage update)
        tokio::task::yield_now().await;

        // Load
        let loaded = manager.load("session1").await.unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.session_id, "session1");
        assert_eq!(loaded.last_message_id, Some("msg123".to_string()));
        assert_eq!(loaded.mode, ReasoningMode::Plan);
        assert_eq!(loaded.mode_state.current_step, 1);
    }

    #[tokio::test]
    async fn test_checkpoint_manager_cache_hit() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let checkpoint = create_test_checkpoint("session2");

        // Save (sync to ensure cache is populated before load)
        manager.save_sync(checkpoint.clone()).await.unwrap();

        // Load should hit cache
        let loaded = manager.load("session2").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().session_id, "session2");
    }

    #[tokio::test]
    async fn test_checkpoint_manager_delete() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let checkpoint = create_test_checkpoint("session3");
        manager.save_sync(checkpoint).await.unwrap();

        // Delete
        manager.delete("session3").await.unwrap();

        // Load should return None
        let loaded = manager.load("session3").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_checkpoint_manager_clear_cache() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let checkpoint = create_test_checkpoint("session4");
        manager.save_sync(checkpoint).await.unwrap();

        // Clear cache
        manager.clear_cache().await;

        // Cache should be empty
        let ids = manager.cached_session_ids().await;
        assert!(ids.is_empty());
    }

    #[test]
    fn test_reasoning_mode_default() {
        let mode = ReasoningMode::default();
        assert_eq!(mode, ReasoningMode::Direct);
    }

    #[test]
    fn test_reasoning_mode_display() {
        assert_eq!(ReasoningMode::Direct.to_string(), "direct");
        assert_eq!(ReasoningMode::Plan.to_string(), "plan");
        assert_eq!(ReasoningMode::Stream.to_string(), "stream");
        assert_eq!(ReasoningMode::Hidden.to_string(), "hidden");
    }

    #[test]
    fn test_pending_message_mark_sent() {
        let mut msg = PendingMessage::new("msg1".to_string(), "content".to_string());
        assert!(!msg.sent);
        msg.mark_sent();
        assert!(msg.sent);
    }

    #[test]
    fn test_session_status_default() {
        let status = SessionStatus::default();
        assert_eq!(status, SessionStatus::Active);
    }

    #[test]
    fn test_session_status_display() {
        assert_eq!(SessionStatus::Active.to_string(), "active");
        assert_eq!(SessionStatus::Archived.to_string(), "archived");
    }

    #[test]
    fn test_checkpoint_builder_new_fields() {
        let now = Utc::now();
        let cp = SessionCheckpoint::new("test-builder".to_string())
            .with_status(SessionStatus::Archived)
            .with_last_message_at(now)
            .with_message_count(42)
            .with_channel("feishu".to_string())
            .with_chat_id("oc_123".to_string());

        assert_eq!(cp.status, SessionStatus::Archived);
        assert_eq!(cp.last_message_at, Some(now));
        assert_eq!(cp.message_count, 42);
        assert_eq!(cp.channel, Some("feishu".to_string()));
        assert_eq!(cp.chat_id, Some("oc_123".to_string()));
    }

    // ===================================================================
    // agent_id / role field tests
    // ===================================================================
    #[test]
    fn test_session_checkpoint_new_agent_id_role_none() {
        let cp = SessionCheckpoint::new("test-session".to_string());
        assert!(
            cp.agent_id.is_none(),
            "agent_id should be None on new checkpoint"
        );
        assert!(cp.role.is_none(), "role should be None on new checkpoint");
    }

    #[test]
    fn test_session_checkpoint_with_agent_id() {
        let cp = SessionCheckpoint::new("test-session".to_string())
            .with_agent_id("agent-eda".to_string());
        assert_eq!(cp.agent_id, Some("agent-eda".to_string()));
        // role should still be None
        assert!(cp.role.is_none());
    }

    #[test]
    fn test_session_checkpoint_with_role_main_agent() {
        use crate::session::persistence::AgentRole;
        let cp = SessionCheckpoint::new("test-session".to_string()).with_role(AgentRole::MainAgent);
        assert_eq!(cp.role, Some(AgentRole::MainAgent));
        // agent_id should still be None
        assert!(cp.agent_id.is_none());
    }

    #[test]
    fn test_session_checkpoint_with_role_sub_agent() {
        use crate::session::persistence::AgentRole;
        let cp = SessionCheckpoint::new("test-session".to_string()).with_role(AgentRole::SubAgent);
        assert_eq!(cp.role, Some(AgentRole::SubAgent));
    }

    #[test]
    fn test_session_checkpoint_with_both_identity_fields() {
        use crate::session::persistence::AgentRole;
        let cp = SessionCheckpoint::new("test-session".to_string())
            .with_agent_id("agent-eda".to_string())
            .with_role(AgentRole::SubAgent);
        assert_eq!(cp.agent_id, Some("agent-eda".to_string()));
        assert_eq!(cp.role, Some(AgentRole::SubAgent));
    }

    #[tokio::test]
    async fn test_checkpoint_manager_new_with_identity_fills_on_save() {
        use crate::session::persistence::AgentRole;
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new_with_identity(
            storage.clone(),
            "agent-eda".to_string(),
            AgentRole::SubAgent,
        );

        // Create checkpoint without identity fields
        let checkpoint =
            SessionCheckpoint::new("session-identity".to_string()).with_message_count(5);

        manager.save_sync(checkpoint).await.unwrap();

        // Load from storage and verify identity was filled by manager
        let storage = manager.storage();
        let loaded = storage.load_checkpoint("session-identity").await.unwrap();
        assert!(loaded.is_some(), "checkpoint should exist in storage");
        let loaded = loaded.unwrap();
        assert_eq!(
            loaded.agent_id,
            Some("agent-eda".to_string()),
            "agent_id should be filled by CheckpointManager"
        );
        assert_eq!(
            loaded.role,
            Some(AgentRole::SubAgent),
            "role should be filled by CheckpointManager"
        );
    }

    #[tokio::test]
    async fn test_checkpoint_manager_new_does_not_fill_identity() {
        let storage = Arc::new(MemoryStorage::new());
        // Using new() (not new_with_identity): agent_id=String::new(), role=MainAgent
        // These get written to storage via with_agent_id/with_role, then loaded back.
        // Empty string "" is stored and treated as empty → loaded as None
        let manager = CheckpointManager::new(storage.clone());

        let checkpoint =
            SessionCheckpoint::new("session-no-identity".to_string()).with_message_count(3);

        manager.save_sync(checkpoint).await.unwrap();

        let loaded = storage
            .load_checkpoint("session-no-identity")
            .await
            .unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        // CheckpointManager::new() uses empty string agent_id, which becomes
        // Some("") in checkpoint, then MemoryStorage saves/restores it as-is.
        // SqliteStorage would convert "" to None via load_checkpoint_inner,
        // but MemoryStorage returns the value directly.
        // For agent_id we expect Some("") for MemoryStorage.
        assert_eq!(
            loaded.agent_id,
            Some(String::new()),
            "agent_id should be empty string with MemoryStorage round-trip"
        );
        // For role, "main_agent" round-trips to Some(MainAgent)
        use crate::session::persistence::AgentRole;
        assert_eq!(
            loaded.role,
            Some(AgentRole::MainAgent),
            "role should be MainAgent with MemoryStorage round-trip"
        );
    }

    #[tokio::test]
    async fn test_checkpoint_manager_save_existing_identity_preserved() {
        use crate::session::persistence::AgentRole;
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new_with_identity(
            storage.clone(),
            "manager-agent".to_string(),
            AgentRole::MainAgent,
        );

        // Checkpoint already has identity set — manager should overwrite with its own
        let checkpoint = SessionCheckpoint::new("session-preserve".to_string())
            .with_agent_id("pre-existing-agent".to_string())
            .with_role(AgentRole::SubAgent);

        manager.save_sync(checkpoint).await.unwrap();

        let loaded = storage.load_checkpoint("session-preserve").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        // Manager identity takes precedence
        assert_eq!(loaded.agent_id, Some("manager-agent".to_string()));
        assert_eq!(loaded.role, Some(AgentRole::MainAgent));
    }

    #[tokio::test]
    async fn test_checkpoint_manager_archive_and_restore() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let checkpoint = create_test_checkpoint("session-archive");
        manager.save_sync(checkpoint.clone()).await.unwrap();

        // Archive
        manager.archive(checkpoint).await.unwrap();

        // Active cache should be cleared
        let loaded = manager.load("session-archive").await.unwrap();
        assert!(loaded.is_none());

        // Archived sessions should contain the ID
        let archived = manager.archived_session_ids().await.unwrap();
        assert!(archived.contains(&"session-archive".to_string()));

        // Restore
        let restored = manager.restore("session-archive").await.unwrap();
        assert!(restored.is_some());
        assert_eq!(restored.unwrap().session_id, "session-archive");

        // After restore, should be loadable from active
        let loaded = manager.load("session-archive").await.unwrap();
        assert!(loaded.is_some());

        // Archived should be empty
        let archived = manager.archived_session_ids().await.unwrap();
        assert!(!archived.contains(&"session-archive".to_string()));
    }

    #[tokio::test]
    async fn test_checkpoint_manager_purge() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let checkpoint = create_test_checkpoint("session-purge");
        manager.save_sync(checkpoint.clone()).await.unwrap();
        manager.archive(checkpoint).await.unwrap();

        // Purge
        manager.purge("session-purge").await.unwrap();

        // Should no longer be in archived
        let archived = manager.archived_session_ids().await.unwrap();
        assert!(!archived.contains(&"session-purge".to_string()));

        // Restore should return None
        let restored = manager.restore("session-purge").await.unwrap();
        assert!(restored.is_none());
    }

    #[test]
    fn test_reasoning_mode_state_operations() {
        let mut state = ReasoningModeState::default();
        assert_eq!(state.current_step, 0);
        assert!(!state.is_complete);

        state.start_step(5);
        assert_eq!(state.current_step, 1);
        assert_eq!(state.total_steps, 5);

        state.add_step_message("Thinking...".to_string());
        assert_eq!(state.step_messages.len(), 1);

        state.complete();
        assert!(state.is_complete);
    }
}
