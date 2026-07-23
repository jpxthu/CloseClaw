use super::*;
use crate::llm_session::SessionMessage;
use crate::persistence::{
    DreamingStatus, PersistenceService, ReasoningLevel, ReasoningMode, ReasoningModeState,
    SessionMode, SessionStatus,
};
use chrono::Utc;
use closeclaw_common::ContentBlock;
use rusqlite::Connection;

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
        message_count: 0,
        platform: None,
        peer_id: None,
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

#[tokio::test]
async fn test_save_load_system_appends_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut checkpoint = create_test_checkpoint("roundtrip-sa");
    checkpoint.system_appends = vec!["append-A".to_string(), "append-B".to_string()];

    // Save
    storage.save_checkpoint(&checkpoint).await.unwrap();

    // Load
    let loaded = storage.load_checkpoint("roundtrip-sa").await.unwrap();
    assert!(loaded.is_some(), "loaded checkpoint should exist");
    let loaded = loaded.unwrap();
    assert_eq!(loaded.system_appends, checkpoint.system_appends);
}

#[tokio::test]
async fn test_load_system_appends_backward_compat() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // 1. Save a checkpoint so the transcript file is created
    let mut checkpoint = create_test_checkpoint("compat-sa");
    checkpoint.system_appends = vec!["should-be-cleared".to_string()];
    storage.save_checkpoint(&checkpoint).await.unwrap();

    // 2. Manually rewrite metadata to remove system_appends key
    //    (simulates an old DB written before the feature existed)
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let metadata_without_appends = json!({
            "reasoning_mode": mode_to_db(&checkpoint.reasoning_mode),
            "mode_state":
                serde_json::to_string(&checkpoint.mode_state).unwrap(),
            "outbound_pending":
                serde_json::to_string(&checkpoint.outbound_pending).unwrap(),
            // intentionally omit "system_appends"
        })
        .to_string();
        conn.execute(
            "UPDATE sessions SET metadata = ?1 WHERE id = ?2",
            params![metadata_without_appends, "compat-sa"],
        )
        .unwrap();
    }

    // 3. Load — system_appends should default to empty Vec
    let loaded = storage.load_checkpoint("compat-sa").await.unwrap();
    assert!(loaded.is_some(), "loaded checkpoint should exist");
    let loaded = loaded.unwrap();
    assert!(
        loaded.system_appends.is_empty(),
        "missing system_appends key in metadata should yield empty Vec"
    );
}

// ===================================================================
// Fallback loading: new columns (platform/peer_id/account_id) NULL
// should fall back to old channel/chat_id columns
// ===================================================================

#[tokio::test]
async fn test_load_fallback_channel_chat_id_to_platform_peer_id() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // 1. Save a checkpoint with platform/peer_id set (new code path)
    let mut checkpoint = create_test_checkpoint("fallback-1");
    checkpoint.platform = Some("feishu".to_string());
    checkpoint.peer_id = Some("oc_abc".to_string());
    checkpoint.account_id = Some("tenant-1".to_string());
    storage.save_checkpoint(&checkpoint).await.unwrap();

    // 2. Simulate old data: set platform/peer_id/account_id to NULL in DB
    //    but keep channel/chat_id with values (as old code would have written)
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE sessions SET channel = 'feishu', chat_id = 'oc_old_chat', platform = NULL, peer_id = NULL, account_id = NULL WHERE id = ?1",
            params!["fallback-1"],
        )
        .unwrap();
    }

    // 3. Load — should fallback to channel/chat_id
    let loaded = storage.load_checkpoint("fallback-1").await.unwrap();
    assert!(loaded.is_some(), "loaded checkpoint should exist");
    let loaded = loaded.unwrap();
    assert_eq!(
        loaded.platform.as_deref(),
        Some("feishu"),
        "platform should fallback to old channel column"
    );
    assert_eq!(
        loaded.peer_id.as_deref(),
        Some("oc_old_chat"),
        "peer_id should fallback to old chat_id column"
    );
    assert!(
        loaded.account_id.is_none(),
        "account_id should be None when new column is NULL"
    );
}

#[tokio::test]
async fn test_load_new_columns_take_precedence_over_old() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // 1. Save a checkpoint
    let mut checkpoint = create_test_checkpoint("fallback-2");
    checkpoint.platform = Some("telegram".to_string());
    checkpoint.peer_id = Some("tg_new_peer".to_string());
    checkpoint.account_id = Some("tenant-2".to_string());
    storage.save_checkpoint(&checkpoint).await.unwrap();

    // 2. Manually set old columns to different values, keep new columns populated
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE sessions SET channel = 'old_feishu', chat_id = 'oc_old_value' WHERE id = ?1",
            params!["fallback-2"],
        )
        .unwrap();
    }

    // 3. Load — new columns should take precedence
    let loaded = storage.load_checkpoint("fallback-2").await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(
        loaded.platform.as_deref(),
        Some("telegram"),
        "new platform column should take precedence over old channel"
    );
    assert_eq!(
        loaded.peer_id.as_deref(),
        Some("tg_new_peer"),
        "new peer_id column should take precedence over old chat_id"
    );
    assert_eq!(
        loaded.account_id.as_deref(),
        Some("tenant-2"),
        "account_id should be loaded from new column"
    );
}

#[tokio::test]
async fn test_load_both_new_and_old_null_yields_none() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // 1. Save a checkpoint with no platform/peer_id/account_id
    let checkpoint = create_test_checkpoint("fallback-3");
    storage.save_checkpoint(&checkpoint).await.unwrap();

    // 2. Verify channel/chat_id are empty strings (default), new columns NULL
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let channel: String = conn
            .query_row(
                "SELECT channel FROM sessions WHERE id = ?1",
                params!["fallback-3"],
                |row| row.get(0),
            )
            .unwrap();
        let chat_id: String = conn
            .query_row(
                "SELECT chat_id FROM sessions WHERE id = ?1",
                params!["fallback-3"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(channel.is_empty(), "channel should be empty string");
        assert!(chat_id.is_empty(), "chat_id should be empty string");
    }

    // 3. Load — both platform and peer_id should be None
    let loaded = storage.load_checkpoint("fallback-3").await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert!(
        loaded.platform.is_none(),
        "platform should be None when both new and old columns are empty"
    );
    assert!(
        loaded.peer_id.is_none(),
        "peer_id should be None when both new and old columns are empty"
    );
    assert!(loaded.account_id.is_none());
}

// ===================================================================
// parent_session_id + depth tests
// ===================================================================

#[tokio::test]
async fn test_save_load_parent_session_id_and_depth() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let cp = SessionCheckpoint::new("spawn-child".into())
        .with_parent_session_id("spawn-parent".into())
        .with_depth(2);
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("spawn-child").await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.parent_session_id.as_deref(), Some("spawn-parent"));
    assert_eq!(loaded.depth, 2);
}

#[tokio::test]
async fn test_parent_session_id_depth_defaults_when_null() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // Save a checkpoint (parent_session_id=NULL, depth=0 by default)
    let cp = create_test_checkpoint("root-session");
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("root-session").await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert!(loaded.parent_session_id.is_none());
    assert_eq!(loaded.depth, 0);
}

// ===================================================================
// list_children_sessions tests
// ===================================================================

#[tokio::test]
async fn test_list_children_sessions_basic() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // Parent
    let mut parent = create_test_checkpoint("parent-db");
    parent.parent_session_id = None;
    storage.save_checkpoint(&parent).await.unwrap();

    // Child 1
    let mut child1 = create_test_checkpoint("child1-db");
    child1.parent_session_id = Some("parent-db".to_string());
    storage.save_checkpoint(&child1).await.unwrap();

    // Child 2
    let mut child2 = create_test_checkpoint("child2-db");
    child2.parent_session_id = Some("parent-db".to_string());
    storage.save_checkpoint(&child2).await.unwrap();

    // Unrelated
    let mut unrelated = create_test_checkpoint("unrelated-db");
    unrelated.parent_session_id = Some("other-parent".to_string());
    storage.save_checkpoint(&unrelated).await.unwrap();

    let mut children = storage.list_children_sessions("parent-db").await.unwrap();
    children.sort();
    assert_eq!(
        children,
        vec!["child1-db".to_string(), "child2-db".to_string()]
    );
}

#[tokio::test]
async fn test_list_children_sessions_no_children() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    storage
        .save_checkpoint(&create_test_checkpoint("no-kids"))
        .await
        .unwrap();
    let children = storage.list_children_sessions("no-kids").await.unwrap();
    assert!(children.is_empty());
}

#[tokio::test]
async fn test_list_children_sessions_after_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut child = create_test_checkpoint("child-del-db");
    child.parent_session_id = Some("parent-del-db".to_string());
    storage.save_checkpoint(&child).await.unwrap();

    let children = storage
        .list_children_sessions("parent-del-db")
        .await
        .unwrap();
    assert_eq!(children, vec!["child-del-db".to_string()]);

    storage.delete_checkpoint("child-del-db").await.unwrap();

    let children = storage
        .list_children_sessions("parent-del-db")
        .await
        .unwrap();
    assert!(children.is_empty());
}

// ===================================================================
// Step 1.3: mined_at tests
// ===================================================================

/// mark_mined() writes a mined_at timestamp within ±5 seconds of the
/// call time.
#[tokio::test]
async fn test_mark_mined_sets_mined_at_timestamp() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("mined-at-sqlite");
    cp.mined = false;
    assert!(cp.mined_at.is_none(), "mined_at should start as None");
    storage.save_checkpoint(&cp).await.unwrap();

    let before = Utc::now().timestamp();
    storage.mark_mined("mined-at-sqlite").await.unwrap();
    let after = Utc::now().timestamp();

    let loaded = storage.load_checkpoint("mined-at-sqlite").await.unwrap();
    assert!(loaded.is_some(), "checkpoint should exist after mark_mined");
    let loaded = loaded.unwrap();
    assert!(loaded.mined, "checkpoint should be marked mined");
    let ts = loaded
        .mined_at
        .expect("mined_at should be Some after mark_mined");
    assert!(
        ts >= before && ts <= after,
        "mined_at ({ts}) should be between {before} and {after}"
    );
}

/// Old sessions (no mined_at column) load with mined_at = None after
/// migration adds the column with no default value.
#[tokio::test]
async fn test_old_session_migration_mined_at_defaults_to_none() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // Save a checkpoint normally — mined_at starts as None
    let mut cp = create_test_checkpoint("old-session");
    cp.mined = false;
    storage.save_checkpoint(&cp).await.unwrap();

    // Simulate old data: set mined_at to NULL directly in the database
    // (as if the row existed before the column was added)
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE sessions SET mined_at = NULL WHERE id = ?1",
            params!["old-session"],
        )
        .unwrap();
    }

    // Load — mined_at should be None for old sessions
    let loaded = storage.load_checkpoint("old-session").await.unwrap();
    assert!(loaded.is_some(), "old session should still load");
    let loaded = loaded.unwrap();
    assert!(!loaded.mined, "old session mined should remain false");
    assert!(
        loaded.mined_at.is_none(),
        "mined_at should be None for old sessions (NULL in DB)"
    );
}

// ===================================================================
// Step 1.3: session_mode persistence tests
// ===================================================================

/// Save → load roundtrip preserves each SessionMode variant.
#[tokio::test]
async fn test_save_load_session_mode_roundtrip_normal() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("sm-normal");
    cp.session_mode = SessionMode::Normal;
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("sm-normal").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().session_mode, SessionMode::Normal);
}

#[tokio::test]
async fn test_save_load_session_mode_roundtrip_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("sm-plan");
    cp.session_mode = SessionMode::Plan;
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("sm-plan").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().session_mode, SessionMode::Plan);
}

#[tokio::test]
async fn test_save_load_session_mode_roundtrip_auto() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("sm-auto");
    cp.session_mode = SessionMode::Auto;
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("sm-auto").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().session_mode, SessionMode::Auto);
}

/// Metadata without session_mode key (simulating old data) falls back to
/// SessionMode::Normal.
#[tokio::test]
async fn test_load_session_mode_backward_compat_missing_key() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // 1. Save a checkpoint so the transcript file is created
    let mut cp = create_test_checkpoint("sm-compat");
    cp.session_mode = SessionMode::Plan;
    storage.save_checkpoint(&cp).await.unwrap();

    // 2. Manually rewrite metadata to remove session_mode key
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let metadata_without_mode = json!({
            "reasoning_mode": mode_to_db(&cp.reasoning_mode),
            "mode_state":
                serde_json::to_string(&cp.mode_state).unwrap(),
            "outbound_pending":
                serde_json::to_string(&cp.outbound_pending).unwrap(),
            "system_appends":
                serde_json::to_string(&cp.system_appends).unwrap(),
            // intentionally omit "session_mode"
        })
        .to_string();
        conn.execute(
            "UPDATE sessions SET metadata = ?1 WHERE id = ?2",
            params![metadata_without_mode, "sm-compat"],
        )
        .unwrap();
    }

    // 3. Load — session_mode should default to Normal
    let loaded = storage.load_checkpoint("sm-compat").await.unwrap();
    assert!(loaded.is_some(), "loaded checkpoint should exist");
    assert_eq!(
        loaded.unwrap().session_mode,
        SessionMode::Normal,
        "missing session_mode key should fall back to Normal"
    );
}

/// Metadata without any metadata field at all falls back to Normal.
#[tokio::test]
async fn test_load_session_mode_backward_compat_no_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // 1. Save a checkpoint so the transcript file is created
    let mut cp = create_test_checkpoint("sm-no-meta");
    cp.session_mode = SessionMode::Auto;
    storage.save_checkpoint(&cp).await.unwrap();

    // 2. Manually set metadata to NULL
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE sessions SET metadata = NULL WHERE id = ?1",
            params!["sm-no-meta"],
        )
        .unwrap();
    }

    // 3. Load — session_mode should default to Normal
    let loaded = storage.load_checkpoint("sm-no-meta").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(
        loaded.unwrap().session_mode,
        SessionMode::Normal,
        "NULL metadata should fall back to Normal"
    );
}

/// Invalid session_mode value in metadata falls back to Normal without panic.
#[tokio::test]
async fn test_load_session_mode_invalid_value_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    // 1. Save a checkpoint so the transcript file is created
    let mut cp = create_test_checkpoint("sm-invalid");
    cp.session_mode = SessionMode::Plan;
    storage.save_checkpoint(&cp).await.unwrap();

    // 2. Manually rewrite metadata with an invalid session_mode value
    {
        let db_path = tmp.path().join("sessions.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let metadata_bad = json!({
            "reasoning_mode": mode_to_db(&cp.reasoning_mode),
            "mode_state":
                serde_json::to_string(&cp.mode_state).unwrap(),
            "outbound_pending":
                serde_json::to_string(&cp.outbound_pending).unwrap(),
            "system_appends":
                serde_json::to_string(&cp.system_appends).unwrap(),
            "session_mode": "nonexistent_mode",
        })
        .to_string();
        conn.execute(
            "UPDATE sessions SET metadata = ?1 WHERE id = ?2",
            params![metadata_bad, "sm-invalid"],
        )
        .unwrap();
    }

    // 3. Load — should not panic and should fallback to Normal
    let loaded = storage.load_checkpoint("sm-invalid").await.unwrap();
    assert!(loaded.is_some(), "should not panic on invalid session_mode");
    assert_eq!(
        loaded.unwrap().session_mode,
        SessionMode::Normal,
        "invalid session_mode should fall back to Normal"
    );
}

// ===================================================================
// Step 1.5: transcript persistence tests
// ===================================================================

/// Save → load roundtrip preserves transcript (pending_messages) in JSONL.
#[tokio::test]
async fn test_transcript_roundtrip_via_jsonl() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("transcript-rt");
    cp.pending_messages = vec![
        SessionMessage {
            role: "user".to_string(),
            content_blocks: vec![ContentBlock::Text("Hello".to_string())],
            timestamp: Utc::now(),
        },
        SessionMessage {
            role: "assistant".to_string(),
            content_blocks: vec![ContentBlock::Text("Hi there".to_string())],
            timestamp: Utc::now(),
        },
    ];
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("transcript-rt").await.unwrap();
    let loaded = loaded.expect("checkpoint should exist");
    assert_eq!(loaded.pending_messages.len(), 2);
    assert_eq!(loaded.pending_messages[0].role, "user");
    assert_eq!(loaded.pending_messages[1].role, "assistant");
}

/// Transcript roundtrip preserves multi-block messages (thinking + text).
#[tokio::test]
async fn test_transcript_roundtrip_multi_block() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("transcript-multi");
    cp.pending_messages = vec![SessionMessage {
        role: "assistant".to_string(),
        content_blocks: vec![
            ContentBlock::Thinking {
                thinking: "reasoning...".to_string(),
                signature: None,
            },
            ContentBlock::Text("final answer".to_string()),
        ],
        timestamp: Utc::now(),
    }];
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("transcript-multi").await.unwrap();
    let loaded = loaded.expect("checkpoint should exist");
    assert_eq!(loaded.pending_messages.len(), 1);
    assert_eq!(loaded.pending_messages[0].content_blocks.len(), 2);
}

/// Metadata JSON does not contain a "transcript" key after save.
#[tokio::test]
async fn test_metadata_does_not_contain_transcript_key() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("no-transcript-key");
    cp.pending_messages = vec![SessionMessage {
        role: "user".to_string(),
        content_blocks: vec![ContentBlock::Text("msg".to_string())],
        timestamp: Utc::now(),
    }];
    storage.save_checkpoint(&cp).await.unwrap();

    // Read raw metadata from SQLite
    let db_path = tmp.path().join("sessions.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    let metadata: String = conn
        .query_row(
            "SELECT metadata FROM sessions WHERE id = ?1",
            params!["no-transcript-key"],
            |row| row.get(0),
        )
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert!(
        v.get("transcript").is_none(),
        "metadata JSON must not contain 'transcript' key"
    );
    // outbound_pending should still be present
    assert!(
        v.get("outbound_pending").is_some(),
        "metadata JSON should contain 'outbound_pending' key"
    );
}

/// Empty transcript saves and loads correctly.
#[tokio::test]
async fn test_empty_transcript_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let cp = create_test_checkpoint("empty-transcript");
    assert!(cp.pending_messages.is_empty());
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage.load_checkpoint("empty-transcript").await.unwrap();
    let loaded = loaded.expect("checkpoint should exist");
    assert!(
        loaded.pending_messages.is_empty(),
        "empty transcript should load as empty Vec"
    );
}

/// Transcript survives archive → restore cycle.
#[tokio::test]
async fn test_transcript_survives_archive_restore() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let mut cp = create_test_checkpoint("arch-restore-transcript");
    cp.pending_messages = vec![SessionMessage {
        role: "user".to_string(),
        content_blocks: vec![ContentBlock::Text("before archive".to_string())],
        timestamp: Utc::now(),
    }];
    storage.save_checkpoint(&cp).await.unwrap();

    // Archive
    storage.archive_checkpoint(&cp).await.unwrap();

    // Verify transcript moved to archived_sessions/
    let archived_path = tmp
        .path()
        .join("archived_sessions")
        .join("arch-restore-transcript.jsonl");
    assert!(archived_path.exists(), "archived JSONL should exist");

    // Restore
    let restored = storage
        .restore_checkpoint("arch-restore-transcript")
        .await
        .unwrap();
    let restored = restored.expect("restored checkpoint should exist");
    assert_eq!(restored.pending_messages.len(), 1);
    assert_eq!(restored.pending_messages[0].role, "user");
}
