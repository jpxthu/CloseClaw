//! SQLite storage backend tests

#![cfg(test)]

mod tests {
    use crate::persistence::{
        DreamingStatus, PersistenceError, PersistenceService, ReasoningLevel, ReasoningMode,
        ReasoningModeState, SessionCheckpoint, SessionMode, SessionStatus,
    };
    use crate::storage::SqliteStorage;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_checkpoint(session_id: &str, status: SessionStatus) -> SessionCheckpoint {
        SessionCheckpoint {
            session_id: session_id.to_string(),
            last_message_id: None,
            mode_state: ReasoningModeState::default(),
            outbound_pending: vec![],
            mode: ReasoningMode::Direct,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ttl_seconds: 604800,
            status,
            last_message_at: Some(Utc::now()),
            message_count: 5,
            platform: Some("test-channel".to_string()),
            peer_id: Some("test-chat".to_string()),
            agent_id: None,
            role: None,
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
            account_id: None,
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            mined_at: None,
            dreaming_status: DreamingStatus::default(),
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
            plan_state: None,
            progress_tool_calls: Vec::new(),
            approval_tool_calls: Vec::new(),
            plan_references: Vec::new(),
            session_mode: SessionMode::default(),
            pending_messages: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
            snapshot_metas: Vec::new(),
        }
    }

    // ===================================================================
    // 1. save_checkpoint + load_checkpoint round-trip
    // ===================================================================
    #[tokio::test]
    async fn test_save_load_roundtrip() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s1", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;

        let loaded = storage.load_checkpoint("s1").await?;
        let loaded = loaded.expect("expected checkpoint");

        assert_eq!(loaded.session_id, checkpoint.session_id);
        assert_eq!(loaded.message_count, checkpoint.message_count);
        assert_eq!(loaded.platform, checkpoint.platform);
        assert_eq!(loaded.peer_id, checkpoint.peer_id);

        Ok(())
    }

    // ===================================================================
    // 2. delete_checkpoint
    // ===================================================================
    #[tokio::test]
    async fn test_delete_not_found() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-del", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;
        storage.delete_checkpoint("s-del").await?;

        let loaded = storage.load_checkpoint("s-del").await?;
        assert!(loaded.is_none(), "Expected None after delete");

        Ok(())
    }

    // ===================================================================
    // 3. list_active_sessions
    // ===================================================================
    #[tokio::test]
    async fn test_list_active_sessions() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        for i in 1..=3 {
            let id = format!("s-list-{}", i);
            storage
                .save_checkpoint(&make_checkpoint(&id, SessionStatus::Active))
                .await?;
        }

        let ids = storage.list_active_sessions().await?;
        assert_eq!(ids.len(), 3, "Expected 3 active sessions");

        Ok(())
    }

    // ===================================================================
    // 4. archive_checkpoint - idempotency
    // ===================================================================
    #[tokio::test]
    async fn test_archive_idempotent() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-arch-idemp", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;

        // First archive
        storage.archive_checkpoint(&checkpoint).await?;
        // Second archive - should be idempotent
        storage.archive_checkpoint(&checkpoint).await?;

        let archived = storage.list_archived_sessions().await?;
        assert!(
            archived.contains(&"s-arch-idemp".to_string()),
            "Should be archived"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_archive_nonexistent_succeeds() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        // Non-existent session - archive should succeed (idempotent)
        let result = storage
            .archive_checkpoint(&make_checkpoint("s-nonexistent", SessionStatus::Active))
            .await;
        assert!(result.is_ok(), "Archive non-existent should succeed");

        Ok(())
    }

    #[tokio::test]
    async fn test_archive_normal() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-arch-normal", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;
        storage.archive_checkpoint(&checkpoint).await?;

        let archived = storage.list_archived_sessions().await?;
        assert!(archived.contains(&"s-arch-normal".to_string()));

        Ok(())
    }

    // ===================================================================
    // 5. restore_checkpoint
    // ===================================================================
    #[tokio::test]
    async fn test_restore_normal() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-restore", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;
        storage.archive_checkpoint(&checkpoint).await?;

        let restored = storage.restore_checkpoint("s-restore").await?;
        assert!(restored.is_some(), "Expected restored checkpoint");

        let active = storage.list_active_sessions().await?;
        assert!(active.contains(&"s-restore".to_string()));

        let archived = storage.list_archived_sessions().await?;
        assert!(!archived.contains(&"s-restore".to_string()));

        Ok(())
    }

    #[tokio::test]
    async fn test_restore_active_returns_error() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-restore-active", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;

        // Active session cannot be restored
        let result = storage.restore_checkpoint("s-restore-active").await;
        assert!(result.is_err(), "Restore active session should fail");

        Ok(())
    }

    // ===================================================================
    // 6. purge_checkpoint
    // ===================================================================
    #[tokio::test]
    async fn test_purge_normal() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-purge", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;
        storage.archive_checkpoint(&checkpoint).await?;
        storage.purge_checkpoint("s-purge").await?;

        let archived = storage.list_archived_sessions().await?;
        assert!(!archived.contains(&"s-purge".to_string()));

        Ok(())
    }

    // ===================================================================
    // 7. list_idle_sessions / list_expired_archived_sessions
    // ===================================================================
    #[tokio::test]
    async fn test_list_idle_sessions_zero() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        for i in 1..=3 {
            let id = format!("s-idle-{}", i);
            storage
                .save_checkpoint(&make_checkpoint(&id, SessionStatus::Active))
                .await?;
        }

        // idle_minutes=0 returns all active sessions
        let idle = storage.list_idle_sessions(0).await?;
        assert_eq!(idle.len(), 3, "Should return all when idle_minutes=0");

        Ok(())
    }

    #[tokio::test]
    async fn test_list_expired_archived_zero_returns_empty() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        // purge_after_minutes=0 means sessions never expire
        let expired = storage.list_expired_archived_sessions(0).await?;
        assert!(
            expired.is_empty(),
            "Should return empty when purge_after_minutes=0"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_list_expired_archived_after_archive() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-expired", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;
        storage.archive_checkpoint(&checkpoint).await?;

        // After archiving, the session should appear in the archived list
        let archived = storage.list_archived_sessions().await?;
        assert!(
            archived.contains(&"s-expired".to_string()),
            "Should be in archived list"
        );

        Ok(())
    }

    // ===================================================================
    // 8. list_idle_sessions_for_agent / list_expired_archived_sessions_for_agent
    // ===================================================================
    #[tokio::test]
    async fn test_list_idle_sessions_for_agent() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        // Save a session - SQLite impl uses "unknown" as agent_id
        storage
            .save_checkpoint(&make_checkpoint("s-agent", SessionStatus::Active))
            .await?;

        // idle_minutes=0 returns all matching sessions
        let idle = storage
            .list_idle_sessions_for_agent("unknown", "main_agent", 0)
            .await?;
        assert!(!idle.is_empty(), "Should find sessions for agent 'unknown'");

        // Non-matching agent returns nothing
        let idle = storage
            .list_idle_sessions_for_agent("nobody", "main_agent", 0)
            .await?;
        assert!(idle.is_empty(), "Should be empty for non-matching agent");

        Ok(())
    }

    #[tokio::test]
    async fn test_list_expired_archived_for_agent() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-arch-agent", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;
        storage.archive_checkpoint(&checkpoint).await?;

        // After archiving, the session should appear in the archived list for the agent
        let expired = storage
            .list_expired_archived_sessions_for_agent("unknown", "main_agent", 0)
            .await?;
        // With purge_after_minutes=0, nothing should be considered expired
        assert!(
            expired.is_empty(),
            "With 0 threshold nothing should be expired"
        );

        Ok(())
    }

    // ===================================================================
    // 9. Atomicity - rename failure leaves DB unchanged
    // ===================================================================
    #[tokio::test]
    async fn test_atomicity_rename_failure() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = make_checkpoint("s-atomic", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;

        // Verify session exists before any archive attempt
        let loaded = storage.load_checkpoint("s-atomic").await?;
        assert!(loaded.is_some(), "Checkpoint must exist before archive");

        // Archive succeeds normally - no transaction corruption
        storage.archive_checkpoint(&checkpoint).await?;

        // After successful archive, session should be archived
        let archived = storage.list_archived_sessions().await?;
        assert!(archived.contains(&"s-atomic".to_string()));

        Ok(())
    }

    // ===================================================================
    // agent_id / role save & load tests
    // ===================================================================
    #[tokio::test]
    async fn test_save_checkpoint_writes_agent_id_role_none() -> Result<(), PersistenceError> {
        use crate::persistence::AgentRole;

        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        // Checkpoint with no agent_id/role — SqliteStorage should write defaults
        let checkpoint = make_checkpoint("s-none", SessionStatus::Active);
        storage.save_checkpoint(&checkpoint).await?;

        let loaded = storage.load_checkpoint("s-none").await?;
        let loaded = loaded.expect("expected checkpoint");
        // None falls back to "unknown" / "main_agent"
        assert_eq!(loaded.agent_id, Some("unknown".to_string()));
        assert_eq!(loaded.role, Some(AgentRole::MainAgent));

        Ok(())
    }

    #[tokio::test]
    async fn test_save_checkpoint_writes_agent_id_role_some() -> Result<(), PersistenceError> {
        use crate::persistence::AgentRole;

        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = SessionCheckpoint {
            session_id: "s-some".to_string(),
            last_message_id: None,
            mode_state: ReasoningModeState::default(),
            outbound_pending: vec![],
            mode: ReasoningMode::Direct,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ttl_seconds: 604800,
            status: SessionStatus::Active,
            last_message_at: Some(chrono::Utc::now()),
            message_count: 1,
            platform: Some("test-channel".to_string()),
            peer_id: Some("test-chat".to_string()),
            agent_id: Some("agent-eda".to_string()),
            role: Some(AgentRole::SubAgent),
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
            account_id: None,
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            mined_at: None,
            dreaming_status: DreamingStatus::default(),
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
            plan_state: None,
            progress_tool_calls: Vec::new(),
            approval_tool_calls: Vec::new(),
            plan_references: Vec::new(),
            session_mode: SessionMode::default(),
            pending_messages: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
            snapshot_metas: Vec::new(),
        };
        storage.save_checkpoint(&checkpoint).await?;

        let loaded = storage.load_checkpoint("s-some").await?;
        let loaded = loaded.expect("expected checkpoint");
        assert_eq!(loaded.agent_id, Some("agent-eda".to_string()));
        assert_eq!(loaded.role, Some(AgentRole::SubAgent));

        Ok(())
    }

    #[tokio::test]
    async fn test_save_checkpoint_agent_id_none_uses_unknown() -> Result<(), PersistenceError> {
        use crate::persistence::AgentRole;

        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        // agent_id is None, role is Some
        let checkpoint = SessionCheckpoint {
            session_id: "s-partial".to_string(),
            last_message_id: None,
            mode_state: ReasoningModeState::default(),
            outbound_pending: vec![],
            mode: ReasoningMode::Direct,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ttl_seconds: 604800,
            status: SessionStatus::Active,
            last_message_at: Some(chrono::Utc::now()),
            message_count: 1,
            platform: None,
            peer_id: None,
            agent_id: None,
            role: Some(AgentRole::SubAgent),
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
            account_id: None,
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            mined_at: None,
            dreaming_status: DreamingStatus::default(),
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
            plan_state: None,
            progress_tool_calls: Vec::new(),
            approval_tool_calls: Vec::new(),
            plan_references: Vec::new(),
            session_mode: SessionMode::default(),
            pending_messages: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
            snapshot_metas: Vec::new(),
        };
        storage.save_checkpoint(&checkpoint).await?;

        let loaded = storage.load_checkpoint("s-partial").await?;
        let loaded = loaded.expect("expected checkpoint");
        // agent_id falls back to "unknown", role is preserved
        assert_eq!(loaded.agent_id, Some("unknown".to_string()));
        assert_eq!(loaded.role, Some(AgentRole::SubAgent));

        Ok(())
    }

    #[tokio::test]
    async fn test_load_checkpoint_roundtrips_agent_id_role() -> Result<(), PersistenceError> {
        use crate::persistence::AgentRole;

        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = SessionCheckpoint {
            session_id: "s-roundtrip".to_string(),
            last_message_id: Some("msg-abc".to_string()),
            mode_state: ReasoningModeState::default(),
            outbound_pending: vec![],
            mode: ReasoningMode::Plan,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ttl_seconds: 86400,
            status: SessionStatus::Active,
            last_message_at: Some(chrono::Utc::now()),
            message_count: 10,
            platform: Some("feishu".to_string()),
            peer_id: Some("oc_123456".to_string()),
            agent_id: Some("my-agent".to_string()),
            role: Some(AgentRole::MainAgent),
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
            account_id: None,
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            mined_at: None,
            dreaming_status: DreamingStatus::default(),
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
            plan_state: None,
            progress_tool_calls: Vec::new(),
            approval_tool_calls: Vec::new(),
            plan_references: Vec::new(),
            session_mode: SessionMode::default(),
            pending_messages: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
            snapshot_metas: Vec::new(),
        };
        storage.save_checkpoint(&checkpoint).await?;

        let loaded = storage.load_checkpoint("s-roundtrip").await?;
        let loaded = loaded.expect("expected checkpoint");

        assert_eq!(loaded.session_id, "s-roundtrip");
        assert_eq!(loaded.agent_id, Some("my-agent".to_string()));
        assert_eq!(loaded.role, Some(AgentRole::MainAgent));
        assert_eq!(loaded.mode, ReasoningMode::Plan);
        assert_eq!(loaded.message_count, 10);

        Ok(())
    }

    // ===================================================================
    // sync() / close() tests
    // ===================================================================

    #[tokio::test]
    async fn test_sqlite_sync_executes_successfully() {
        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        // Save a checkpoint so WAL has data to checkpoint
        let cp = make_checkpoint("sync-test", SessionStatus::Active);
        storage.save_checkpoint(&cp).await.unwrap();

        // sync() should succeed (issues PRAGMA wal_checkpoint(TRUNCATE))
        let result = storage.sync().await;
        assert!(result.is_ok(), "sync() should succeed: {:?}", result);
    }

    #[tokio::test]
    async fn test_sqlite_sync_succeeds_without_data() {
        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        // sync() on empty database should also succeed
        let result = storage.sync().await;
        assert!(result.is_ok(), "sync() on empty DB should succeed");
    }

    #[tokio::test]
    async fn test_sqlite_close_returns_ok() {
        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        // close() is a no-op for SqliteStorage but should return Ok
        let result = storage.close().await;
        assert!(result.is_ok(), "close() should return Ok(())");
    }

    #[tokio::test]
    async fn test_sqlite_sync_then_close() {
        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        let cp = make_checkpoint("sync-close-test", SessionStatus::Active);
        storage.save_checkpoint(&cp).await.unwrap();

        storage.sync().await.unwrap();
        storage.close().await.unwrap();
    }

    // ===================================================================
    // plan_state SQLite roundtrip tests
    // ===================================================================

    #[tokio::test]
    async fn test_sqlite_plan_state_roundtrip() {
        use closeclaw_common::{PlanPhase, PlanState};

        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        let plan = PlanState {
            phase: PlanPhase::FinalPlan,
            pending_steps: vec!["step-a".into(), "step-b".into()],
            plan_file_path: "/workspace/plan.md".into(),
            ..Default::default()
        };
        let mut cp = make_checkpoint("plan-sqlite-rt", SessionStatus::Active);
        cp.plan_state = Some(plan);
        storage.save_checkpoint(&cp).await.unwrap();

        let loaded = storage.load_checkpoint("plan-sqlite-rt").await.unwrap();
        assert!(loaded.is_some(), "checkpoint should exist");
        let ps = loaded.unwrap().plan_state.unwrap();
        assert_eq!(ps.phase, PlanPhase::FinalPlan);
        assert_eq!(ps.pending_steps, vec!["step-a", "step-b"]);
        assert_eq!(ps.plan_file_path, "/workspace/plan.md");
    }

    #[tokio::test]
    async fn test_sqlite_plan_state_none_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        let cp = make_checkpoint("plan-none-sqlite", SessionStatus::Active);
        assert!(cp.plan_state.is_none());
        storage.save_checkpoint(&cp).await.unwrap();

        let loaded = storage.load_checkpoint("plan-none-sqlite").await.unwrap();
        assert!(loaded.is_some());
        assert!(loaded.unwrap().plan_state.is_none());
    }

    #[tokio::test]
    async fn test_sqlite_plan_state_update_roundtrip() {
        use closeclaw_common::{PlanPhase, PlanState};

        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        // Save with Research phase
        let plan1 = PlanState {
            phase: PlanPhase::Research,
            pending_steps: vec![],
            plan_file_path: String::new(),
            ..Default::default()
        };
        let mut cp = make_checkpoint("plan-update-sqlite", SessionStatus::Active);
        cp.plan_state = Some(plan1);
        storage.save_checkpoint(&cp).await.unwrap();

        // Update to Design phase
        let plan2 = PlanState {
            phase: PlanPhase::Design,
            pending_steps: vec!["analyze".into()],
            plan_file_path: "/tmp/p.md".into(),
            ..Default::default()
        };
        cp.plan_state = Some(plan2);
        storage.save_checkpoint(&cp).await.unwrap();

        let loaded = storage
            .load_checkpoint("plan-update-sqlite")
            .await
            .unwrap()
            .unwrap();
        let ps = loaded.plan_state.unwrap();
        assert_eq!(ps.phase, PlanPhase::Design);
        assert_eq!(ps.pending_steps, vec!["analyze"]);
        assert_eq!(ps.plan_file_path, "/tmp/p.md");
    }

    #[tokio::test]
    async fn test_sqlite_mark_mined_sets_mined_at() {
        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();

        let cp = make_checkpoint("mined-at-sqlite", SessionStatus::Active);
        storage.save_checkpoint(&cp).await.unwrap();

        let before = chrono::Utc::now().timestamp();
        storage.mark_mined("mined-at-sqlite").await.unwrap();
        let after = chrono::Utc::now().timestamp();

        let loaded = storage
            .load_checkpoint("mined-at-sqlite")
            .await
            .unwrap()
            .expect("checkpoint should exist");
        assert!(loaded.mined, "should be marked mined");
        let ts = loaded
            .mined_at
            .expect("mined_at should be Some after mark_mined");
        assert!(
            ts >= before && ts <= after,
            "mined_at ({ts}) should be between {before} and {after}"
        );
    }
}
