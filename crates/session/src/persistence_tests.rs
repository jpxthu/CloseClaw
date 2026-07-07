//! Tests for persistence module

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;

    use crate::checkpoint_manager::CheckpointManager;
    use crate::persistence::{
        DreamingStatus, PendingMessage, PersistenceService, ReasoningMode, ReasoningModeState,
        SessionCheckpoint, SessionMode, SessionStatus,
    };
    use crate::storage::memory::MemoryStorage;

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
            .with_platform("feishu".to_string())
            .with_peer_id("oc_123".to_string());

        assert_eq!(cp.status, SessionStatus::Archived);
        assert_eq!(cp.last_message_at, Some(now));
        assert_eq!(cp.message_count, 42);
        assert_eq!(cp.platform, Some("feishu".to_string()));
        assert_eq!(cp.peer_id, Some("oc_123".to_string()));
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
        use crate::persistence::AgentRole;
        let cp = SessionCheckpoint::new("test-session".to_string()).with_role(AgentRole::MainAgent);
        assert_eq!(cp.role, Some(AgentRole::MainAgent));
        // agent_id should still be None
        assert!(cp.agent_id.is_none());
    }

    #[test]
    fn test_session_checkpoint_with_role_sub_agent() {
        use crate::persistence::AgentRole;
        let cp = SessionCheckpoint::new("test-session".to_string()).with_role(AgentRole::SubAgent);
        assert_eq!(cp.role, Some(AgentRole::SubAgent));
    }

    #[test]
    fn test_session_checkpoint_with_both_identity_fields() {
        use crate::persistence::AgentRole;
        let cp = SessionCheckpoint::new("test-session".to_string())
            .with_agent_id("agent-eda".to_string())
            .with_role(AgentRole::SubAgent);
        assert_eq!(cp.agent_id, Some("agent-eda".to_string()));
        assert_eq!(cp.role, Some(AgentRole::SubAgent));
    }

    #[tokio::test]
    async fn test_checkpoint_manager_new_with_identity_fills_on_save() {
        use crate::persistence::AgentRole;
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
        use crate::persistence::AgentRole;
        assert_eq!(
            loaded.role,
            Some(AgentRole::MainAgent),
            "role should be MainAgent with MemoryStorage round-trip"
        );
    }

    #[tokio::test]
    async fn test_checkpoint_manager_save_existing_identity_preserved() {
        use crate::persistence::AgentRole;
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

    // ── thread_id tests ───────────────────────────────────────────────────────

    #[test]
    fn test_checkpoint_thread_id_roundtrip() {
        let cp = SessionCheckpoint::new("s1".into()).with_thread_id("omt_abc123".into());
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.thread_id.as_deref(), Some("omt_abc123"));
    }

    #[test]
    fn test_checkpoint_thread_id_default_none() {
        let cp = SessionCheckpoint::new("s2".into());
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value.as_object_mut().unwrap().remove("thread_id");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            parsed.thread_id.is_none(),
            "old data without thread_id should default to None"
        );
    }

    #[test]
    fn test_checkpoint_with_thread_id_builder() {
        let cp = SessionCheckpoint::new("s3".into()).with_thread_id("omt_xyz".into());
        assert_eq!(cp.thread_id.as_deref(), Some("omt_xyz"));
    }

    #[test]
    fn test_checkpoint_thread_id_none_roundtrip() {
        let cp = SessionCheckpoint::new("s4".into());
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert!(parsed.thread_id.is_none());
    }

    // ── account_id tests ───────────────────────────────────────────────────────

    #[test]
    fn test_checkpoint_account_id_default_none() {
        let cp = SessionCheckpoint::new("s5".into());
        assert!(cp.account_id.is_none(), "account_id should default to None");
    }

    #[test]
    fn test_checkpoint_with_account_id_builder() {
        let cp = SessionCheckpoint::new("s6".into()).with_account_id("tenant-42".to_string());
        assert_eq!(cp.account_id.as_deref(), Some("tenant-42"));
    }

    #[test]
    fn test_checkpoint_account_id_roundtrip() {
        let cp = SessionCheckpoint::new("s7".into()).with_account_id("tenant-99".to_string());
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.account_id.as_deref(), Some("tenant-99"));
    }

    #[test]
    fn test_checkpoint_account_id_missing_json_defaults_none() {
        // Simulate old JSON without account_id field — should deserialize to None
        let cp = SessionCheckpoint::new("s8".into());
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value.as_object_mut().unwrap().remove("account_id");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            parsed.account_id.is_none(),
            "old data without account_id should default to None"
        );
    }

    #[test]
    fn test_checkpoint_platform_peer_id_account_id_none_roundtrip() {
        let cp = SessionCheckpoint::new("s9".into());
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert!(parsed.platform.is_none());
        assert!(parsed.peer_id.is_none());
        assert!(parsed.account_id.is_none());
    }

    // ── parent_session_id + depth tests ───────────────────────────────────────

    #[test]
    fn test_checkpoint_parent_session_id_and_depth_default() {
        let cp = SessionCheckpoint::new("s-parent-depth".into());
        assert!(
            cp.parent_session_id.is_none(),
            "parent_session_id should default to None"
        );
        assert_eq!(cp.depth, 0, "depth should default to 0");
    }

    #[test]
    fn test_checkpoint_with_parent_session_id_and_depth_builder() {
        let cp = SessionCheckpoint::new("s-builder".into())
            .with_parent_session_id("parent-id".to_string())
            .with_depth(2);
        assert_eq!(cp.parent_session_id.as_deref(), Some("parent-id"));
        assert_eq!(cp.depth, 2);
    }

    #[test]
    fn test_checkpoint_parent_session_id_depth_roundtrip() {
        let cp = SessionCheckpoint::new("s-roundtrip".into())
            .with_parent_session_id("p1".to_string())
            .with_depth(3);
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.parent_session_id.as_deref(), Some("p1"));
        assert_eq!(parsed.depth, 3);
    }

    #[test]
    fn test_checkpoint_parent_session_id_depth_missing_json_defaults() {
        // Simulate old JSON without parent_session_id and depth fields —
        // should deserialize to None / 0 via #[serde(default)]
        let cp = SessionCheckpoint::new("s-old-json".into())
            .with_parent_session_id("old-parent".to_string())
            .with_depth(5);
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value
            .as_object_mut()
            .unwrap()
            .remove("parent_session_id");
        json_value.as_object_mut().unwrap().remove("depth");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            parsed.parent_session_id.is_none(),
            "old data without parent_session_id should default to None"
        );
        assert_eq!(
            parsed.depth, 0,
            "old data without depth should default to 0"
        );
    }

    #[test]
    fn test_checkpoint_parent_session_id_none_roundtrip() {
        let cp = SessionCheckpoint::new("s-none-parent".into());
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert!(parsed.parent_session_id.is_none());
        assert_eq!(parsed.depth, 0);
    }

    #[test]
    fn test_checkpoint_parent_session_id_depth_zero_roundtrip() {
        let cp = SessionCheckpoint::new("s-depth-zero".into())
            .with_parent_session_id("root-parent".to_string())
            .with_depth(0);
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.parent_session_id.as_deref(), Some("root-parent"));
        assert_eq!(parsed.depth, 0);
    }

    #[tokio::test]
    async fn test_list_archived_unmined_sessions() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();

        // Archived and mined=false
        let mut cp1 = create_test_checkpoint("archived-unmined");
        cp1.status = SessionStatus::Archived;
        cp1.mined = false;
        storage.archive_checkpoint(&cp1).await.unwrap();

        // Archived and mined=true
        let mut cp2 = create_test_checkpoint("archived-mined");
        cp2.status = SessionStatus::Archived;
        cp2.mined = true;
        storage.archive_checkpoint(&cp2).await.unwrap();

        let unmined = storage.list_archived_unmined_sessions().await.unwrap();
        assert_eq!(unmined, vec!["archived-unmined".to_string()]);
    }

    #[tokio::test]
    async fn test_mark_mined_updates_field() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();
        let cp = create_test_checkpoint("mark-mined-test");
        storage.save_checkpoint(&cp).await.unwrap();

        storage.mark_mined("mark-mined-test").await.unwrap();

        let loaded = storage.load_checkpoint("mark-mined-test").await.unwrap();
        assert!(loaded.is_some());
        assert!(loaded.unwrap().mined);
    }

    #[tokio::test]
    async fn test_list_mined_undreamt_sessions() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();

        // mined=true, dreaming_status=Pending
        let mut cp1 = create_test_checkpoint("mined-undreamt");
        cp1.mined = true;
        cp1.dreaming_status = DreamingStatus::Pending;
        storage.save_checkpoint(&cp1).await.unwrap();

        // mined=true, dreaming_status=Completed
        let mut cp2 = create_test_checkpoint("mined-dreamt");
        cp2.mined = true;
        cp2.dreaming_status = DreamingStatus::Completed;
        storage.save_checkpoint(&cp2).await.unwrap();

        // mined=false
        let cp3 = create_test_checkpoint("not-mined");
        storage.save_checkpoint(&cp3).await.unwrap();

        let undreamt = storage.list_mined_undreamt_sessions().await.unwrap();
        assert_eq!(undreamt, vec!["mined-undreamt".to_string()]);
    }

    #[tokio::test]
    async fn test_update_dreaming_status() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();
        let cp = create_test_checkpoint("dreaming-status-test");
        storage.save_checkpoint(&cp).await.unwrap();

        storage
            .update_dreaming_status("dreaming-status-test", DreamingStatus::InLight)
            .await
            .unwrap();

        let loaded = storage
            .load_checkpoint("dreaming-status-test")
            .await
            .unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().dreaming_status, DreamingStatus::InLight);
    }

    // ===================================================================
    // sync() / close() default trait implementation tests
    // ===================================================================

    /// MemoryStorage uses the trait default sync() — should return Ok(()).
    #[tokio::test]
    async fn test_memory_storage_sync_default_returns_ok() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();
        let result = storage.sync().await;
        assert!(result.is_ok(), "default sync() should return Ok(())");
    }

    /// MemoryStorage uses the trait default close() — should return Ok(()).
    #[tokio::test]
    async fn test_memory_storage_close_default_returns_ok() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();
        let result = storage.close().await;
        assert!(result.is_ok(), "default close() should return Ok(())");
    }

    /// sync() and close() are independent — calling both succeeds.
    #[tokio::test]
    async fn test_sync_then_close_both_succeed() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();
        storage.sync().await.unwrap();
        storage.close().await.unwrap();
    }

    // ===================================================================
    // plan_state tests
    // ===================================================================

    #[test]
    fn test_checkpoint_plan_state_default_none() {
        let cp = SessionCheckpoint::new("s-plan-default".into());
        assert!(cp.plan_state.is_none());
    }

    #[test]
    fn test_checkpoint_with_plan_state_builder() {
        use closeclaw_common::{PlanPhase, PlanState};

        let plan = PlanState {
            phase: PlanPhase::Design,
            pending_steps: vec!["step1".into(), "step2".into()],
            plan_file_path: "/tmp/plan.md".into(),
            execution_steps: vec![],
            current_step: None,
        };
        let cp = SessionCheckpoint::new("s-plan-builder".into()).with_plan_state(plan.clone());
        let ps = cp.plan_state.unwrap();
        assert_eq!(ps.phase, PlanPhase::Design);
        assert_eq!(ps.pending_steps, vec!["step1", "step2"]);
        assert_eq!(ps.plan_file_path, "/tmp/plan.md");
    }

    #[test]
    fn test_checkpoint_plan_state_roundtrip() {
        use closeclaw_common::{PlanPhase, PlanState};

        let plan = PlanState {
            phase: PlanPhase::Review,
            pending_steps: vec!["a".into()],
            plan_file_path: "/p.md".into(),
            execution_steps: vec![],
            current_step: None,
        };
        let cp = SessionCheckpoint::new("s-plan-rt".into()).with_plan_state(plan);
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        let ps = parsed.plan_state.unwrap();
        assert_eq!(ps.phase, PlanPhase::Review);
        assert_eq!(ps.pending_steps, vec!["a"]);
        assert_eq!(ps.plan_file_path, "/p.md");
    }

    #[test]
    fn test_checkpoint_plan_state_none_roundtrip() {
        let cp = SessionCheckpoint::new("s-plan-none".into());
        assert!(cp.plan_state.is_none());
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert!(parsed.plan_state.is_none());
    }

    #[test]
    fn test_checkpoint_plan_state_missing_json_defaults_none() {
        // Simulate old JSON without plan_state field
        let cp = SessionCheckpoint::new("s-old-json-plan".into()).with_message_count(5);
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value.as_object_mut().unwrap().remove("plan_state");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            parsed.plan_state.is_none(),
            "old data without plan_state should default to None"
        );
    }

    #[tokio::test]
    async fn test_checkpoint_manager_save_load_with_plan_state() {
        use closeclaw_common::{PlanPhase, PlanState};

        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let plan = PlanState {
            phase: PlanPhase::Interview,
            pending_steps: vec!["todo1".into()],
            plan_file_path: "/workspace/plan.md".into(),
            execution_steps: vec![],
            current_step: None,
        };
        let checkpoint = SessionCheckpoint::new("session-plan".into()).with_plan_state(plan);

        manager.save_sync(checkpoint).await.unwrap();

        let loaded = manager.load("session-plan").await.unwrap();
        assert!(loaded.is_some());
        let ps = loaded.unwrap().plan_state.unwrap();
        assert_eq!(ps.phase, PlanPhase::Interview);
        assert_eq!(ps.pending_steps, vec!["todo1"]);
        assert_eq!(ps.plan_file_path, "/workspace/plan.md");
    }

    #[tokio::test]
    async fn test_checkpoint_plan_state_survives_compaction_simulation() {
        use crate::storage::memory::MemoryStorage;
        use closeclaw_common::{PlanPhase, PlanState};

        // Simulate compaction protection flow:
        // 1. Save checkpoint with plan_state
        // 2. Load it (simulating pre-compact state)
        // 3. Re-save with touch() (simulating save_checkpoint_after_compact)
        // 4. Reload and verify plan_state unchanged
        let storage = MemoryStorage::new();
        let plan = PlanState {
            phase: PlanPhase::Design,
            pending_steps: vec!["s1".into(), "s2".into()],
            plan_file_path: "/plan.md".into(),
            execution_steps: vec![],
            current_step: None,
        };
        let cp = SessionCheckpoint::new("compact-test".into()).with_plan_state(plan);
        storage.save_checkpoint(&cp).await.unwrap();

        // Simulate compaction: load -> touch -> save
        let mut loaded = storage
            .load_checkpoint("compact-test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.plan_state.as_ref().unwrap().phase, PlanPhase::Design);
        loaded.touch();
        storage.save_checkpoint(&loaded).await.unwrap();

        // Verify plan_state survived
        let after = storage
            .load_checkpoint("compact-test")
            .await
            .unwrap()
            .unwrap();
        let ps = after.plan_state.unwrap();
        assert_eq!(ps.phase, PlanPhase::Design);
        assert_eq!(ps.pending_steps, vec!["s1", "s2"]);
        assert_eq!(ps.plan_file_path, "/plan.md");
    }

    #[tokio::test]
    async fn test_checkpoint_plan_state_none_compaction_works() {
        use crate::storage::memory::MemoryStorage;

        let storage = MemoryStorage::new();
        let cp = SessionCheckpoint::new("compact-none-test".into());
        assert!(cp.plan_state.is_none());
        storage.save_checkpoint(&cp).await.unwrap();

        // Simulate compaction: load -> touch -> save
        let mut loaded = storage
            .load_checkpoint("compact-none-test")
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.plan_state.is_none());
        loaded.touch();
        storage.save_checkpoint(&loaded).await.unwrap();

        let after = storage
            .load_checkpoint("compact-none-test")
            .await
            .unwrap()
            .unwrap();
        assert!(
            after.plan_state.is_none(),
            "plan_state should remain None after compaction"
        );
    }

    // ===================================================================
    // session_mode persistence tests
    // ===================================================================

    #[test]
    fn test_checkpoint_session_mode_default_is_normal() {
        let cp = SessionCheckpoint::new("s-mode-default".into());
        assert_eq!(cp.session_mode, SessionMode::Normal);
    }

    #[test]
    fn test_checkpoint_with_session_mode_builder() {
        let cp =
            SessionCheckpoint::new("s-mode-builder".into()).with_session_mode(SessionMode::Auto);
        assert_eq!(cp.session_mode, SessionMode::Auto);
    }

    #[test]
    fn test_checkpoint_session_mode_roundtrip() {
        for mode in [SessionMode::Normal, SessionMode::Plan, SessionMode::Auto] {
            let cp = SessionCheckpoint::new(format!("s-mode-rt-{}", mode)).with_session_mode(mode);
            let json = serde_json::to_string(&cp).unwrap();
            let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.session_mode, mode);
        }
    }

    #[test]
    fn test_checkpoint_session_mode_missing_json_defaults_normal() {
        let cp = SessionCheckpoint::new("s-mode-old".into())
            .with_session_mode(SessionMode::Auto)
            .with_message_count(5);
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value.as_object_mut().unwrap().remove("session_mode");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert_eq!(
            parsed.session_mode,
            SessionMode::Normal,
            "old data without session_mode should default to Normal"
        );
    }

    #[tokio::test]
    async fn test_checkpoint_manager_save_load_with_session_mode() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = CheckpointManager::new(storage);

        let checkpoint =
            SessionCheckpoint::new("session-mode-save".into()).with_session_mode(SessionMode::Auto);
        manager.save_sync(checkpoint).await.unwrap();

        let loaded = manager.load("session-mode-save").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().session_mode, SessionMode::Auto);
    }

    #[tokio::test]
    async fn test_checkpoint_session_mode_persists_through_memory_storage() {
        let storage = MemoryStorage::new();
        let cp = SessionCheckpoint::new("s-mode-mem".into())
            .with_session_mode(SessionMode::Plan)
            .with_message_count(3);
        storage.save_checkpoint(&cp).await.unwrap();

        let loaded = storage.load_checkpoint("s-mode-mem").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().session_mode, SessionMode::Plan);
    }

    #[tokio::test]
    async fn test_checkpoint_session_mode_update_via_builder() {
        let storage = MemoryStorage::new();
        let cp = SessionCheckpoint::new("s-mode-update".into())
            .with_session_mode(SessionMode::Normal)
            .with_message_count(1);
        storage.save_checkpoint(&cp).await.unwrap();

        // Load, change mode, save again
        let mut loaded = storage
            .load_checkpoint("s-mode-update")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.session_mode, SessionMode::Normal);
        loaded.session_mode = SessionMode::Auto;
        loaded.touch();
        storage.save_checkpoint(&loaded).await.unwrap();

        let reloaded = storage
            .load_checkpoint("s-mode-update")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.session_mode, SessionMode::Auto);
    }

    #[test]
    fn test_checkpoint_session_mode_not_affected_by_reasoning_mode() {
        // SessionMode and ReasoningMode are orthogonal — changing one
        // should not affect the other.
        let cp = SessionCheckpoint::new("s-mode-ortho".into())
            .with_session_mode(SessionMode::Auto)
            .with_mode(ReasoningMode::Plan);
        assert_eq!(cp.session_mode, SessionMode::Auto);
        assert_eq!(cp.mode, ReasoningMode::Plan);

        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_mode, SessionMode::Auto);
        assert_eq!(parsed.mode, ReasoningMode::Plan);
    }
}
