use super::*;
use crate::session::persistence::{
    PersistenceService, ReasoningLevel, ReasoningMode, ReasoningModeState, SessionStatus,
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
        channel: None,
        chat_id: None,
        agent_id: None,
        role: None,
        reasoning_level: ReasoningLevel::default(),
        system_appends: Vec::new(),
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
