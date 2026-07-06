use super::*;
use crate::persistence::{
    DreamingStatus, PersistenceService, ReasoningLevel, ReasoningMode, ReasoningModeState,
    SessionStatus,
};
use chrono::Utc;

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
        pending_messages: Vec::new(),
        mode: ReasoningMode::Plan,
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
            "mode": mode_to_db(&checkpoint.mode),
            "mode_state":
                serde_json::to_string(&checkpoint.mode_state).unwrap(),
            "pending_messages":
                serde_json::to_string(&checkpoint.pending_messages).unwrap(),
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
