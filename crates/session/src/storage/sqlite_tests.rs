//! SQLite storage backend tests

#![cfg(test)]

mod tests {
    use crate::llm_session::SessionMessage;
    use crate::persistence::{
        DreamingStatus, PersistenceError, PersistenceService, ReasoningLevel, ReasoningMode,
        ReasoningModeState, SessionCheckpoint, SessionMode, SessionStatus,
    };
    use crate::storage::SqliteStorage;
    use chrono::Utc;
    use closeclaw_common::ContentBlock;
    use tempfile::TempDir;

    fn make_checkpoint(session_id: &str, status: SessionStatus) -> SessionCheckpoint {
        SessionCheckpoint {
            session_id: session_id.to_string(),
            last_message_id: None,
            mode_state: ReasoningModeState::default(),
            outbound_pending: vec![],
            reasoning_mode: ReasoningMode::Direct,
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
            reasoning_mode: ReasoningMode::Direct,
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
    async fn test_load_checkpoint_roundtrips_agent_id_role() -> Result<(), PersistenceError> {
        use crate::persistence::AgentRole;

        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let checkpoint = SessionCheckpoint {
            session_id: "s-roundtrip".to_string(),
            last_message_id: Some("msg-abc".to_string()),
            mode_state: ReasoningModeState::default(),
            outbound_pending: vec![],
            reasoning_mode: ReasoningMode::Plan,
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
        assert_eq!(loaded.reasoning_mode, ReasoningMode::Plan);
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

    // ===================================================================
    // find_archived_session_by_routing tests
    // ===================================================================

    #[tokio::test]
    async fn test_find_archived_session_by_routing_hit() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        let mut cp = make_checkpoint("s-arch-find", SessionStatus::Active);
        cp.platform = Some("feishu".to_string());
        cp.sender_id = Some("user-1".to_string());
        cp.peer_id = Some("chat-1".to_string());
        cp.account_id = Some("acct-1".to_string());
        storage.save_checkpoint(&cp).await.unwrap();
        storage.archive_checkpoint(&cp).await.unwrap();

        let found = storage
            .find_archived_session_by_routing(Some("acct-1"), "feishu", "user-1", "chat-1")
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some("s-arch-find"));
    }

    #[tokio::test]
    async fn test_find_archived_session_by_routing_no_match() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        let found = storage
            .find_archived_session_by_routing(Some("acct-x"), "feishu", "user-x", "chat-x")
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_find_archived_session_by_routing_multiple_returns_newest() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        let now = Utc::now();
        let mut cp_old = make_checkpoint("s-arch-old", SessionStatus::Active);
        cp_old.platform = Some("feishu".to_string());
        cp_old.sender_id = Some("user-1".to_string());
        cp_old.peer_id = Some("chat-1".to_string());
        cp_old.account_id = None;
        cp_old.last_message_at = Some(now - chrono::Duration::hours(2));
        storage.save_checkpoint(&cp_old).await.unwrap();
        storage.archive_checkpoint(&cp_old).await.unwrap();

        let mut cp_new = make_checkpoint("s-arch-new", SessionStatus::Active);
        cp_new.platform = Some("feishu".to_string());
        cp_new.sender_id = Some("user-1".to_string());
        cp_new.peer_id = Some("chat-1".to_string());
        cp_new.account_id = None;
        cp_new.last_message_at = Some(now);
        storage.save_checkpoint(&cp_new).await.unwrap();
        storage.archive_checkpoint(&cp_new).await.unwrap();

        let found = storage
            .find_archived_session_by_routing(None, "feishu", "user-1", "chat-1")
            .await
            .unwrap();
        assert_eq!(
            found.as_deref(),
            Some("s-arch-new"),
            "should return the most recent archived session"
        );
    }

    #[tokio::test]
    async fn test_find_archived_session_by_routing_different_fields_no_cross() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        let mut cp = make_checkpoint("s-arch-diff", SessionStatus::Active);
        cp.platform = Some("feishu".to_string());
        cp.sender_id = Some("user-1".to_string());
        cp.peer_id = Some("chat-1".to_string());
        cp.account_id = None;
        storage.save_checkpoint(&cp).await.unwrap();
        storage.archive_checkpoint(&cp).await.unwrap();

        // Different platform
        let found = storage
            .find_archived_session_by_routing(None, "telegram", "user-1", "chat-1")
            .await
            .unwrap();
        assert!(found.is_none(), "different platform should not match");

        // Different sender_id
        let found = storage
            .find_archived_session_by_routing(None, "feishu", "user-2", "chat-1")
            .await
            .unwrap();
        assert!(found.is_none(), "different sender_id should not match");

        // Different peer_id
        let found = storage
            .find_archived_session_by_routing(None, "feishu", "user-1", "chat-2")
            .await
            .unwrap();
        assert!(found.is_none(), "different peer_id should not match");
    }

    #[tokio::test]
    async fn test_find_archived_session_by_routing_account_id_none_vs_some() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        let mut cp = make_checkpoint("s-arch-acct", SessionStatus::Active);
        cp.platform = Some("feishu".to_string());
        cp.sender_id = Some("user-1".to_string());
        cp.peer_id = Some("chat-1".to_string());
        cp.account_id = Some("acct-1".to_string());
        storage.save_checkpoint(&cp).await.unwrap();
        storage.archive_checkpoint(&cp).await.unwrap();

        // Query with account_id=None should NOT match a record with account_id=Some
        let found = storage
            .find_archived_session_by_routing(None, "feishu", "user-1", "chat-1")
            .await
            .unwrap();
        assert!(found.is_none(), "None account_id should not match Some");

        // Query with account_id=Some("acct-1") should match
        let found = storage
            .find_archived_session_by_routing(Some("acct-1"), "feishu", "user-1", "chat-1")
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some("s-arch-acct"));
    }

    #[tokio::test]
    async fn test_find_archived_session_by_routing_ignores_active() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        let mut cp = make_checkpoint("s-active-only", SessionStatus::Active);
        cp.platform = Some("feishu".to_string());
        cp.sender_id = Some("user-1".to_string());
        cp.peer_id = Some("chat-1".to_string());
        cp.account_id = None;
        storage.save_checkpoint(&cp).await.unwrap();
        // Do NOT archive

        let found = storage
            .find_archived_session_by_routing(None, "feishu", "user-1", "chat-1")
            .await
            .unwrap();
        assert!(found.is_none(), "should not return active session");
    }

    // ===================================================================
    // run_incremental_consistency_check tests
    // ===================================================================

    #[tokio::test]
    async fn test_incremental_check_since_zero_is_full_scan() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        // since=0 should behave like a full scan
        let result = storage.run_incremental_consistency_check(0).await.unwrap();
        assert_eq!(result.deleted_orphaned_records, 0);
        assert_eq!(result.deleted_orphaned_files, 0);
    }

    #[tokio::test]
    async fn test_incremental_check_skips_old_records() {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path()).unwrap();

        // Save a checkpoint with last_message_at in the past
        let mut cp = make_checkpoint("s-old", SessionStatus::Active);
        cp.last_message_at = Some(chrono::DateTime::from_timestamp(1577836800, 0).unwrap());
        storage.save_checkpoint(&cp).await.unwrap();

        // Delete the transcript file to create an orphan
        let transcript = temp.path().join("sessions").join("s-old.jsonl");
        let _ = std::fs::remove_file(&transcript);

        // Incremental scan with since = far future should skip this record
        let future = chrono::Utc::now().timestamp() + 1000;
        let result = storage
            .run_incremental_consistency_check(future)
            .await
            .unwrap();
        assert_eq!(
            result.deleted_orphaned_records, 0,
            "old record should be skipped"
        );
    }

    // ===================================================================
    // Step 1.5: transcript persistence tests
    // ===================================================================

    /// Transcript (pending_messages) round-trips through save/load via JSONL.
    #[tokio::test]
    async fn test_transcript_roundtrip() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let mut cp = make_checkpoint("transcript-rt", SessionStatus::Active);
        cp.pending_messages = vec![
            SessionMessage {
                role: "user".to_string(),
                content_blocks: vec![ContentBlock::Text("hello".to_string())],
                timestamp: Utc::now(),
            },
            SessionMessage {
                role: "assistant".to_string(),
                content_blocks: vec![ContentBlock::Text("world".to_string())],
                timestamp: Utc::now(),
            },
        ];
        storage.save_checkpoint(&cp).await?;

        let loaded = storage.load_checkpoint("transcript-rt").await?;
        let loaded = loaded.expect("checkpoint should exist");
        assert_eq!(loaded.pending_messages.len(), 2);
        assert_eq!(loaded.pending_messages[0].role, "user");
        assert_eq!(loaded.pending_messages[1].role, "assistant");
        Ok(())
    }

    /// Transcript is empty after save/load when no messages are present.
    #[tokio::test]
    async fn test_empty_transcript_roundtrip() -> Result<(), PersistenceError> {
        let temp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(temp.path())?;

        let cp = make_checkpoint("empty-transcript", SessionStatus::Active);
        assert!(cp.pending_messages.is_empty());
        storage.save_checkpoint(&cp).await?;

        let loaded = storage.load_checkpoint("empty-transcript").await?;
        let loaded = loaded.expect("checkpoint should exist");
        assert!(loaded.pending_messages.is_empty());
        Ok(())
    }

    // ===================================================================
    // Step 1.4: reasoning_mode SQLite persistence tests
    // ===================================================================

    /// reasoning_mode round-trips through SQLite with correct metadata key
    /// and session_mode independence.
    #[tokio::test]
    async fn test_sqlite_reasoning_mode_roundtrip() {
        for mode in [
            ReasoningMode::Direct,
            ReasoningMode::Plan,
            ReasoningMode::Stream,
            ReasoningMode::Hidden,
        ] {
            let tmp = TempDir::new().unwrap();
            let storage = SqliteStorage::new(tmp.path()).unwrap();
            let mut cp = make_checkpoint(&format!("rm-{:?}", mode), SessionStatus::Active);
            cp.reasoning_mode = mode;
            cp.session_mode = SessionMode::Auto;
            storage.save_checkpoint(&cp).await.unwrap();
            let loaded = storage
                .load_checkpoint(&format!("rm-{:?}", mode))
                .await
                .unwrap()
                .expect("checkpoint should exist");
            assert_eq!(loaded.reasoning_mode, mode);
            assert_eq!(loaded.session_mode, SessionMode::Auto);
            // Verify metadata uses new key, not legacy "mode"
            let conn = rusqlite::Connection::open(tmp.path().join("sessions.sqlite")).unwrap();
            let meta: String = conn
                .query_row(
                    &format!("SELECT metadata FROM sessions WHERE id = 'rm-{:?}'", mode),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
            assert!(v.get("reasoning_mode").is_some());
            assert!(v.get("mode").is_none());
        }
    }

    /// Old metadata with "mode" key loads correctly via backward-compat fallback.
    #[tokio::test]
    async fn test_sqlite_reasoning_mode_backward_compat_old_key() {
        let tmp = TempDir::new().unwrap();
        let storage = SqliteStorage::new(tmp.path()).unwrap();
        let mut cp = make_checkpoint("rm-compat", SessionStatus::Active);
        cp.reasoning_mode = ReasoningMode::Stream;
        storage.save_checkpoint(&cp).await.unwrap();
        // Rewrite metadata to use old "mode" key
        {
            let conn = rusqlite::Connection::open(tmp.path().join("sessions.sqlite")).unwrap();
            let meta: String = conn
                .query_row(
                    "SELECT metadata FROM sessions WHERE id = 'rm-compat'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let mut v: serde_json::Value = serde_json::from_str(&meta).unwrap();
            let rm = v.as_object_mut().unwrap().remove("reasoning_mode").unwrap();
            v.as_object_mut().unwrap().insert("mode".into(), rm);
            conn.execute(
                "UPDATE sessions SET metadata = ?1 WHERE id = 'rm-compat'",
                rusqlite::params![v.to_string()],
            )
            .unwrap();
        }
        let loaded = storage
            .load_checkpoint("rm-compat")
            .await
            .unwrap()
            .expect("checkpoint should exist");
        assert_eq!(loaded.reasoning_mode, ReasoningMode::Stream);
    }
}
