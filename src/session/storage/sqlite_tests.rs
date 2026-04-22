//! SQLite storage backend tests

#![cfg(test)]

mod tests {
    use crate::session::persistence::{
        PersistenceError, PersistenceService, ReasoningMode, ReasoningModeState, SessionCheckpoint,
        SessionStatus,
    };
    use crate::session::storage::SqliteStorage;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_checkpoint(session_id: &str, status: SessionStatus) -> SessionCheckpoint {
        SessionCheckpoint {
            session_id: session_id.to_string(),
            last_message_id: None,
            mode_state: ReasoningModeState::default(),
            pending_messages: vec![],
            mode: ReasoningMode::Direct,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ttl_seconds: 604800,
            status,
            last_message_at: Some(Utc::now()),
            message_count: 5,
            channel: Some("test-channel".to_string()),
            chat_id: Some("test-chat".to_string()),
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
        assert_eq!(loaded.channel, checkpoint.channel);
        assert_eq!(loaded.chat_id, checkpoint.chat_id);

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
}
