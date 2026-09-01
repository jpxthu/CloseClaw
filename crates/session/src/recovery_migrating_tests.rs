//! Step 1.6: Recovery scan for migrating sessions
//!
//! Tests that migrating sessions are correctly handled during recovery:
//! - With pending operations → restore to active
//! - Without pending operations → complete the archive

use crate::persistence::{
    DreamingStatus, PendingOperation, PendingOperationDetail, PendingOperationStatus,
    PendingOperationType, PersistenceError, PersistenceService, ReasoningLevel, ReasoningMode,
    ReasoningModeState, SessionCheckpoint, SessionMode, SessionStatus,
};
use crate::recovery::SessionRecoveryService;
use crate::storage::memory::MemoryStorage;
use chrono::Utc;
use std::sync::Arc;

fn create_test_checkpoint(session_id: &str) -> SessionCheckpoint {
    SessionCheckpoint {
        session_id: session_id.to_string(),
        last_message_id: Some("msg123".to_string()),
        mode_state: ReasoningModeState {
            current_step: 1,
            total_steps: 3,
            step_messages: vec!["Step 1".to_string()],
            is_complete: false,
        },
        outbound_pending: Vec::new(),
        reasoning_mode: ReasoningMode::Plan,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        ttl_seconds: 604800,
        status: SessionStatus::Active,
        last_message_at: None,
        last_user_activity_at: None,
        message_count: 0,
        platform: None,
        peer_id: None,
        account_id: None,
        agent_id: None,
        role: None,
        reasoning_level: ReasoningLevel::default(),
        system_appends: Vec::new(),
        thread_id: None,
        reply_ref: None,
        sender_id: None,
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
        snapshot_metas: Vec::new(),
        workflow_run: None,
    }
}

/// Helper: create a checkpoint and place it in the migrating map of MemoryStorage.
/// This simulates a session stuck in migrating status after a crash.
async fn setup_migrating_session(
    storage: &MemoryStorage,
    session_id: &str,
    pending_ops: Vec<PendingOperation>,
) {
    let mut cp = create_test_checkpoint(session_id);
    cp.status = SessionStatus::Active;
    cp.pending_operations = pending_ops;

    // Save as active first
    storage.save_checkpoint(&cp).await.unwrap();

    // Archive it (moves from active to archived)
    storage.archive_checkpoint(&cp).await.unwrap();

    // Now restore it (moves from archived to active)
    storage.restore_checkpoint(session_id).await.unwrap();

    // Re-save as active with the pending ops
    cp.status = SessionStatus::Active;
    storage.save_checkpoint(&cp).await.unwrap();

    // Save as migrating (adds to migrating map)
    cp.status = SessionStatus::Migrating;
    storage.save_checkpoint(&cp).await.unwrap();

    // Remove from active map so only migrating has it
    storage.remove_active(session_id).await;
}

/// Migrating session with pending operations should be restored to active
/// during recovery scan.
#[tokio::test]
async fn test_recovery_scan_migrating_with_pending_ops() -> Result<(), PersistenceError> {
    let storage = Arc::new(MemoryStorage::new());
    let now = Utc::now();

    // Set up a migrating session with a pending tool call
    setup_migrating_session(
        &storage,
        "mig-dirty",
        vec![PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "op_mig_1".into(),
            op_type: PendingOperationType::ToolCall,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "exec".into(),
                args_summary: r#"{"command":"ls"}"#.into(),
            },
            created_at: now,
        }],
    )
    .await;

    // Verify it's in the migrating list
    let migrating = storage.list_migrating_sessions().await?;
    assert!(
        migrating.contains(&"mig-dirty".to_string()),
        "session should be in migrating list"
    );

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await?;

    // Should be recovered
    assert!(
        report.recovered.contains(&"mig-dirty".to_string()),
        "migrating session with pending ops should be recovered"
    );

    // Should be in active sessions now
    let active = storage.list_active_sessions().await?;
    assert!(
        active.contains(&"mig-dirty".to_string()),
        "session should be restored to active"
    );

    // Should no longer be in migrating list
    let migrating = storage.list_migrating_sessions().await?;
    assert!(
        !migrating.contains(&"mig-dirty".to_string()),
        "session should no longer be in migrating list"
    );

    // Should be marked as dirty
    assert!(
        report.dirty_sessions.contains(&"mig-dirty".to_string()),
        "restored session should be dirty"
    );

    // Recovery notification should be stored
    let loaded = storage.load_checkpoint("mig-dirty").await?.unwrap();
    assert!(
        loaded.recovery_notification.is_some(),
        "recovery notification should be stored"
    );

    Ok(())
}

/// Migrating session without pending operations should have its archive
/// completed during recovery scan (moved to archived).
#[tokio::test]
async fn test_recovery_scan_migrating_without_pending_ops() -> Result<(), PersistenceError> {
    let storage = Arc::new(MemoryStorage::new());

    // Set up a migrating session with no pending operations
    setup_migrating_session(&storage, "mig-clean", vec![]).await;

    // Verify it's in the migrating list
    let migrating = storage.list_migrating_sessions().await?;
    assert!(
        migrating.contains(&"mig-clean".to_string()),
        "session should be in migrating list"
    );

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await?;

    // Should NOT be recovered (archive was completed, not restored)
    assert!(
        !report.recovered.contains(&"mig-clean".to_string()),
        "migrating session without pending ops should NOT be recovered"
    );

    // Should be in archived sessions now
    let archived = storage.list_archived_sessions().await?;
    assert!(
        archived.contains(&"mig-clean".to_string()),
        "session should be archived after recovery"
    );

    // Should no longer be in migrating list
    let migrating = storage.list_migrating_sessions().await?;
    assert!(
        !migrating.contains(&"mig-clean".to_string()),
        "session should no longer be in migrating list"
    );

    // Should NOT be in active sessions
    let active = storage.list_active_sessions().await?;
    assert!(
        !active.contains(&"mig-clean".to_string()),
        "clean migrating session should not be restored to active"
    );

    Ok(())
}
