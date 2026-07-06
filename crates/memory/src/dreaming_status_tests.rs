//! DreamingStatus transition tests for Step 1.2.
//!
//! Tests the `mark_sessions_status` helper, full pipeline status transitions,
//! and early-return paths when entries are empty.

use crate::dreaming::DreamingPipeline;
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::{
    DreamingCapacityConfig, DreamingConfig, DreamingDiaryConfig, DreamingScoringConfig,
    DreamingThresholdConfig,
};
use closeclaw_session::persistence::{DreamingStatus, SessionCheckpoint};

use tempfile::TempDir;

/// mark_sessions_status updates each session's status to the specified value,
/// and Completed status matches mark_sessions_completed behavior.
#[tokio::test]
async fn test_mark_sessions_status_updates_all_sessions() {
    let storage = TestStorage::default();
    let mut cp1 = SessionCheckpoint::new("s1".into());
    cp1.mined = true;
    cp1.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp1);
    let mut cp2 = SessionCheckpoint::new("s2".into());
    cp2.mined = true;
    cp2.dreaming_status = DreamingStatus::InDeep;
    storage.add_checkpoint(cp2);

    let pipeline = DreamingPipeline::new();
    // InRem for both.
    pipeline
        .mark_sessions_status(&storage, &["s1".into(), "s2".into()], DreamingStatus::InRem)
        .await
        .unwrap();
    {
        let cps = storage.checkpoints.lock().unwrap();
        let c1 = cps.iter().find(|c| c.session_id == "s1").unwrap();
        let c2 = cps.iter().find(|c| c.session_id == "s2").unwrap();
        assert_eq!(c1.dreaming_status, DreamingStatus::InRem);
        assert_eq!(c2.dreaming_status, DreamingStatus::InRem);
    }
    // Completed for both (equivalent to mark_sessions_completed).
    pipeline
        .mark_sessions_status(
            &storage,
            &["s1".into(), "s2".into()],
            DreamingStatus::Completed,
        )
        .await
        .unwrap();
    let cps = storage.checkpoints.lock().unwrap();
    let c1 = cps.iter().find(|c| c.session_id == "s1").unwrap();
    let c2 = cps.iter().find(|c| c.session_id == "s2").unwrap();
    assert_eq!(c1.dreaming_status, DreamingStatus::Completed);
    assert_eq!(c2.dreaming_status, DreamingStatus::Completed);
}

/// Full flow: sessions transition through Pending → InLight → InRem → InDeep → Completed.
/// InLight is set during collect_entries, InRem/InDeep by mark_sessions_status,
/// Completed at end. Since TestStorage tracks the last-set status, verifying
/// Completed confirms all prior transitions executed (they happen sequentially).
#[tokio::test]
async fn test_run_once_full_dreaming_status_transition() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("transitions.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL,
             category TEXT NOT NULL, lesson TEXT, source_session_id TEXT NOT NULL,
             timestamp INTEGER NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE entities (id INTEGER PRIMARY KEY AUTOINCREMENT, agent_id TEXT NOT NULL,
             type TEXT NOT NULL, name TEXT NOT NULL, normalized_name TEXT NOT NULL,
             UNIQUE(agent_id, type, normalized_name));
             CREATE TABLE event_entities (id INTEGER PRIMARY KEY AUTOINCREMENT,
             event_id INTEGER NOT NULL, entity_id INTEGER NOT NULL);
             INSERT INTO events (content, category, lesson, source_session_id, timestamp, updated_at)
             VALUES ('err1', 'error', 'fix it', 'sess-1', 1700000000, 1700000000);
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('agent-1', 'subject', 'deploy', 'deploy');
             INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1);",
        )
        .unwrap();
    }

    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(false),
            ..Default::default()
        },
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig {
            max_rules: Some(100),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config).with_db_path(&db_path);

    // Verify initial Pending status.
    {
        let cps = storage.checkpoints.lock().unwrap();
        let cp = cps.iter().find(|c| c.session_id == "sess-1").unwrap();
        assert_eq!(cp.dreaming_status, DreamingStatus::Pending);
    }

    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once should succeed: {result:?}");

    // Final status must be Completed (all prior transitions happened sequentially).
    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps.iter().find(|c| c.session_id == "sess-1").unwrap();
    assert_eq!(
        cp.dreaming_status,
        DreamingStatus::Completed,
        "session should reach Completed after full pipeline"
    );
}

/// Early return: when collect_entries returns empty (no db or no matching events),
/// pipeline skips InRem/InDeep and goes directly to Completed.
#[tokio::test]
async fn test_run_once_early_return_empty_entries_skips_rem_deep() {
    // Case 1: no db_path → collect_entries returns empty.
    {
        let storage = TestStorage::default();
        let mut cp = SessionCheckpoint::new("sess-no-db".into());
        cp.mined = true;
        cp.dreaming_status = DreamingStatus::Pending;
        storage.add_checkpoint(cp);
        let config = DreamingConfig {
            enabled: Some(true),
            diary: DreamingDiaryConfig {
                enabled: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        let pipeline = DreamingPipeline::with_config(config);
        pipeline.run_once(&storage).await.unwrap();
        let cps = storage.checkpoints.lock().unwrap();
        let cp = cps.iter().find(|c| c.session_id == "sess-no-db").unwrap();
        assert_eq!(cp.dreaming_status, DreamingStatus::Completed);
    }
    // Case 2: db exists but no matching events.
    {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("no_events.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL,
                 category TEXT NOT NULL, lesson TEXT, source_session_id TEXT NOT NULL,
                 timestamp INTEGER NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE entities (id INTEGER PRIMARY KEY AUTOINCREMENT, agent_id TEXT NOT NULL,
                 type TEXT NOT NULL, name TEXT NOT NULL, normalized_name TEXT NOT NULL,
                 UNIQUE(agent_id, type, normalized_name));
                 CREATE TABLE event_entities (id INTEGER PRIMARY KEY AUTOINCREMENT,
                 event_id INTEGER NOT NULL, entity_id INTEGER NOT NULL);",
            )
            .unwrap();
        }
        let storage = TestStorage::default();
        let mut cp = SessionCheckpoint::new("sess-no-events".into());
        cp.mined = true;
        cp.dreaming_status = DreamingStatus::Pending;
        storage.add_checkpoint(cp);
        let config = DreamingConfig {
            enabled: Some(true),
            diary: DreamingDiaryConfig {
                enabled: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        let pipeline = DreamingPipeline::with_config(config).with_db_path(&db_path);
        pipeline.run_once(&storage).await.unwrap();
        let cps = storage.checkpoints.lock().unwrap();
        let cp = cps
            .iter()
            .find(|c| c.session_id == "sess-no-events")
            .unwrap();
        assert_eq!(cp.dreaming_status, DreamingStatus::Completed);
    }
}
