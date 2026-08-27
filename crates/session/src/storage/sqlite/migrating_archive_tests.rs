//! Step 1.6: Migrating state archive tests
//!
//! Tests for two-step crash-safe archive, migrating idempotency,
//! and transcript file location under migrating status.

use crate::llm_session::SessionMessage;
use crate::persistence::{PersistenceError, PersistenceService, SessionStatus};
use crate::storage::SqliteStorage;
use closeclaw_common::ContentBlock;
use tempfile::TempDir;

fn make_checkpoint_with_transcript(
    session_id: &str,
    status: SessionStatus,
) -> crate::persistence::SessionCheckpoint {
    crate::persistence::SessionCheckpoint {
        session_id: session_id.to_string(),
        last_message_id: None,
        mode_state: crate::persistence::ReasoningModeState::default(),
        outbound_pending: vec![],
        reasoning_mode: crate::persistence::ReasoningMode::Direct,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        ttl_seconds: 604800,
        status,
        last_message_at: Some(chrono::Utc::now()),
        last_user_activity_at: None,
        message_count: 5,
        platform: Some("test-channel".to_string()),
        peer_id: Some("test-chat".to_string()),
        agent_id: None,
        role: None,
        reasoning_level: crate::persistence::ReasoningLevel::default(),
        system_appends: Vec::new(),
        thread_id: None,
        sender_id: None,
        account_id: None,
        parent_session_id: None,
        depth: 0,
        effective_max_spawn_depth: None,
        mined: false,
        mined_at: None,
        dreaming_status: crate::persistence::DreamingStatus::default(),
        pending_operations: Vec::new(),
        recovery_notification: None,
        pending_tool_failures: Vec::new(),
        verbosity_level: closeclaw_common::VerbosityLevel::default(),
        plan_state: None,
        progress_tool_calls: Vec::new(),
        approval_tool_calls: Vec::new(),
        plan_references: Vec::new(),
        session_mode: crate::persistence::SessionMode::default(),
        pending_messages: vec![
            SessionMessage {
                role: "user".to_string(),
                content_blocks: vec![ContentBlock::Text("hello".to_string())],
                timestamp: chrono::Utc::now(),
            },
            SessionMessage {
                role: "assistant".to_string(),
                content_blocks: vec![ContentBlock::Text("world".to_string())],
                timestamp: chrono::Utc::now(),
            },
        ],
        label: None,
        communication_config: None,
        snapshot_metas: Vec::new(),
        workflow_run: None,
    }
}

/// Helper: read the status column directly from the SQLite DB.
fn read_status_from_db(data_dir: &std::path::Path, session_id: &str) -> String {
    let conn = rusqlite::Connection::open(data_dir.join("sessions.sqlite")).unwrap();
    conn.query_row(
        "SELECT status FROM sessions WHERE id = ?1",
        rusqlite::params![session_id],
        |row| row.get::<_, String>(0),
    )
    .unwrap()
}

/// Verify archive performs two-step state transition:
/// active → migrating → archived, and transcript ends up in archived_sessions/.
#[tokio::test]
async fn test_archive_two_step_migrating_state() -> Result<(), PersistenceError> {
    let temp = TempDir::new().unwrap();
    let storage = SqliteStorage::new(temp.path())?;

    let cp = make_checkpoint_with_transcript("two-step", SessionStatus::Active);
    storage.save_checkpoint(&cp).await?;

    // Transcript starts in sessions/
    let src = temp.path().join("sessions").join("two-step.jsonl");
    assert!(
        src.exists(),
        "transcript should be in sessions/ before archive"
    );

    // Archive — do_archive runs Step A (migrating) → Step B (move) → Step C (archived)
    storage.archive_checkpoint(&cp).await?;

    // Verify final status is archived
    let status = read_status_from_db(temp.path(), "two-step");
    assert_eq!(status, "archived", "final status should be 'archived'");

    // Transcript should now be in archived_sessions/
    let dst = temp.path().join("archived_sessions").join("two-step.jsonl");
    assert!(
        dst.exists(),
        "transcript should be in archived_sessions/ after archive"
    );
    assert!(
        !src.exists(),
        "transcript should no longer be in sessions/ after archive"
    );

    Ok(())
}

/// Simulate a crash between Step A (migrating) and Step C (archived):
/// manually set status to migrating and move transcript back to sessions/,
/// then verify archive_checkpoint completes the interrupted archive.
#[tokio::test]
async fn test_archive_migrating_crash_recovery() -> Result<(), PersistenceError> {
    let temp = TempDir::new().unwrap();
    let storage = SqliteStorage::new(temp.path())?;

    let cp = make_checkpoint_with_transcript("crash-recovery", SessionStatus::Active);
    storage.save_checkpoint(&cp).await?;

    // Simulate Step A completed but Step B/C didn't:
    // 1. Set status to "migrating" directly in DB
    {
        let conn = rusqlite::Connection::open(temp.path().join("sessions.sqlite")).unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'migrating' WHERE id = ?1",
            rusqlite::params!["crash-recovery"],
        )
        .unwrap();
    }

    // 2. Verify transcript is still in sessions/ (Step B didn't happen)
    let src = temp.path().join("sessions").join("crash-recovery.jsonl");
    assert!(src.exists(), "transcript should still be in sessions/");

    // Call archive_checkpoint — should detect migrating status and complete
    storage.archive_checkpoint(&cp).await?;

    // Verify final state
    let status = read_status_from_db(temp.path(), "crash-recovery");
    assert_eq!(status, "archived", "should complete to 'archived'");

    let dst = temp
        .path()
        .join("archived_sessions")
        .join("crash-recovery.jsonl");
    assert!(
        dst.exists(),
        "transcript should be moved to archived_sessions/"
    );
    assert!(!src.exists(), "transcript should no longer be in sessions/");

    Ok(())
}

/// Verify idempotency: archiving a session already in migrating status
/// completes the archive without error.
#[tokio::test]
async fn test_migrating_idempotent_archive() -> Result<(), PersistenceError> {
    let temp = TempDir::new().unwrap();
    let storage = SqliteStorage::new(temp.path())?;

    let cp = make_checkpoint_with_transcript("idemp-migrating", SessionStatus::Active);
    storage.save_checkpoint(&cp).await?;

    // Set status to migrating directly (simulate partial archive)
    {
        let conn = rusqlite::Connection::open(temp.path().join("sessions.sqlite")).unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'migrating' WHERE id = ?1",
            rusqlite::params!["idemp-migrating"],
        )
        .unwrap();
    }

    // First archive call — completes the interrupted archive
    storage.archive_checkpoint(&cp).await?;
    let status1 = read_status_from_db(temp.path(), "idemp-migrating");
    assert_eq!(status1, "archived");

    // Second archive call — idempotent (already archived)
    storage.archive_checkpoint(&cp).await?;
    let status2 = read_status_from_db(temp.path(), "idemp-migrating");
    assert_eq!(status2, "archived");

    Ok(())
}

/// Verify load_checkpoint finds transcript in the correct location
/// when status is migrating: prefer archived_sessions/ over sessions/.
#[tokio::test]
async fn test_load_checkpoint_migrating_transcript_location() -> Result<(), PersistenceError> {
    let temp = TempDir::new().unwrap();
    let storage = SqliteStorage::new(temp.path())?;

    // --- Case 1: transcript in sessions/ (file not yet moved) ---
    let cp1 = make_checkpoint_with_transcript("mig-in-sessions", SessionStatus::Active);
    storage.save_checkpoint(&cp1).await?;

    // Set status to migrating; transcript stays in sessions/
    {
        let conn = rusqlite::Connection::open(temp.path().join("sessions.sqlite")).unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'migrating' WHERE id = ?1",
            rusqlite::params!["mig-in-sessions"],
        )
        .unwrap();
    }

    let loaded = storage.load_checkpoint("mig-in-sessions").await?;
    let loaded = loaded.expect("should find checkpoint with transcript in sessions/");
    assert_eq!(loaded.session_id, "mig-in-sessions");
    assert_eq!(loaded.status, SessionStatus::Migrating);
    assert_eq!(loaded.pending_messages.len(), 2);

    // --- Case 2: transcript in archived_sessions/ (file already moved) ---
    let cp2 = make_checkpoint_with_transcript("mig-in-archived", SessionStatus::Active);
    storage.save_checkpoint(&cp2).await?;

    // Manually move transcript to archived_sessions/
    let src = temp.path().join("sessions").join("mig-in-archived.jsonl");
    let dst = temp
        .path()
        .join("archived_sessions")
        .join("mig-in-archived.jsonl");
    std::fs::rename(&src, &dst).unwrap();

    // Set status to migrating
    {
        let conn = rusqlite::Connection::open(temp.path().join("sessions.sqlite")).unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'migrating' WHERE id = ?1",
            rusqlite::params!["mig-in-archived"],
        )
        .unwrap();
    }

    let loaded = storage.load_checkpoint("mig-in-archived").await?;
    let loaded = loaded.expect("should find checkpoint with transcript in archived_sessions/");
    assert_eq!(loaded.session_id, "mig-in-archived");
    assert_eq!(loaded.status, SessionStatus::Migrating);
    assert_eq!(loaded.pending_messages.len(), 2);

    Ok(())
}
