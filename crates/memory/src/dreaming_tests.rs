//! Additional unit tests for DreamingPipeline.
//!
//! Complements the inline tests in dreaming.rs with tests that require
//! mock PersistenceService interactions.

use crate::dreaming::{DreamingPipeline, EntityGroup, EntryCategory, MemoryEntry};
use crate::dreaming_llm::PromotedGroupInfo;
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::{DreamingConfig, DreamingDiaryConfig};
use closeclaw_session::persistence::{DreamingStatus, SessionCheckpoint};

use tempfile::TempDir;

/// Dreaming pipeline does not reprocess sessions already marked Completed.
#[tokio::test]
async fn test_dreaming_does_not_reprocess_completed() {
    let storage = TestStorage::default();

    // Session is mined=true but dreaming_status=Completed → should be skipped.
    let mut cp = SessionCheckpoint::new("sess-already-done".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Completed;
    storage.add_checkpoint(cp);

    let pipeline = DreamingPipeline::new();
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once should succeed: {result:?}");

    // Verify the session was NOT reprocessed (dreaming_status unchanged).
    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps
        .iter()
        .find(|c| c.session_id == "sess-already-done")
        .unwrap();
    assert_eq!(
        cp.dreaming_status,
        DreamingStatus::Completed,
        "Completed session should not be reprocessed"
    );
}

/// Pipeline processes mined but not yet dreamt sessions.
#[tokio::test]
async fn test_dreaming_processes_mined_undreamt_sessions() {
    let storage = TestStorage::default();

    // mined=true, dreaming_status=Pending → should be processed.
    let mut cp = SessionCheckpoint::new("sess-pending".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig::default(),
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once should succeed: {result:?}");

    // The pipeline updates dreaming_status to Completed after processing.
    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps.iter().find(|c| c.session_id == "sess-pending").unwrap();
    assert_eq!(
        cp.dreaming_status,
        DreamingStatus::Completed,
        "dreaming should mark session as Completed"
    );
}

/// Pipeline returns Ok immediately when there are no sessions to process.
#[tokio::test]
async fn test_dreaming_empty_storage_returns_ok() {
    let storage = TestStorage::default();
    let pipeline = DreamingPipeline::new();
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok());
}

/// Pipeline returns Ok immediately when dreaming is disabled.
#[tokio::test]
async fn test_dreaming_disabled_skips_processing() {
    let storage = TestStorage::default();

    let mut cp = SessionCheckpoint::new("sess-pending".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let config = DreamingConfig {
        enabled: Some(false),
        diary: DreamingDiaryConfig::default(),
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once should succeed: {result:?}");

    // Session should NOT be reprocessed (dreaming_status unchanged).
    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps.iter().find(|c| c.session_id == "sess-pending").unwrap();
    assert_eq!(
        cp.dreaming_status,
        DreamingStatus::Pending,
        "disabled dreaming should not process sessions"
    );
}

/// Helper to create a MemoryEntry for testing.
fn make_entry(
    category: EntryCategory,
    body: &str,
    session_id: &str,
    minutes_ago: i64,
) -> MemoryEntry {
    let timestamp = chrono::Utc::now() - chrono::Duration::minutes(minutes_ago);
    MemoryEntry {
        category,
        body: body.to_string(),
        timestamp,
        source_session_id: session_id.to_string(),
        lesson: None,
        tags: Vec::new(),
        score: 0.0,
        event_id: 0,
        entity_type: String::new(),
        entity_name: String::new(),
        updated_at: timestamp,
    }
}

fn make_entry_with_lesson(
    category: EntryCategory,
    body: &str,
    session_id: &str,
    lesson: &str,
) -> MemoryEntry {
    let timestamp = chrono::Utc::now();
    MemoryEntry {
        category,
        body: body.to_string(),
        timestamp,
        source_session_id: session_id.to_string(),
        lesson: Some(lesson.to_string()),
        tags: Vec::new(),
        score: 0.0,
        event_id: 0,
        entity_type: String::new(),
        entity_name: String::new(),
        updated_at: timestamp,
    }
}

/// Helper to create an EntityGroup for testing.
fn make_group(entries: Vec<MemoryEntry>, entity_name: &str) -> EntityGroup {
    EntityGroup {
        entity_name: entity_name.to_string(),
        entity_type: "subject".to_string(),
        entries,
        frequency: 1,
        cross_agent_count: 1,
        score: 0.0,
    }
}

/// Dream Diary writes when enabled, skips when disabled, auto-creates dir.
#[tokio::test]
async fn test_dream_diary_enabled_disabled_and_dir() {
    // Enabled: writes file with expected content.
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(true),
            path: Some(diary_path.clone()),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);
    let promoted = vec![PromotedGroupInfo {
        entity_name: "deploy".into(),
        entity_type: "subject".into(),
        lessons: vec![
            "dark mode preferred".into(),
            "verify before deploying".into(),
        ],
    }];
    pipeline.write_dream_diary(&promoted, None).await.unwrap();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let diary_file = tmp.path().join(format!("{}.md", date));
    assert!(diary_file.exists());
    let content = std::fs::read_to_string(&diary_file).unwrap();
    assert!(content.contains("dark mode preferred"));
    assert!(content.contains("Promoted 2 lessons"));

    // Disabled: no file created.
    let tmp2 = TempDir::new().unwrap();
    let config2 = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(false),
            path: Some(tmp2.path().to_str().unwrap().to_string()),
        },
        ..Default::default()
    };
    let p2 = DreamingPipeline::with_config(config2);
    p2.write_dream_diary(&promoted, None).await.unwrap();
    assert!(tmp2.path().read_dir().unwrap().next().is_none());

    // Custom path with nested dir: auto-created.
    let tmp3 = TempDir::new().unwrap();
    let custom = tmp3.path().join("custom/diary");
    let config3 = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(true),
            path: Some(custom.to_str().unwrap().to_string()),
        },
        ..Default::default()
    };
    DreamingPipeline::with_config(config3)
        .write_dream_diary(&promoted, None)
        .await
        .unwrap();
    assert!(custom.exists());
}

/// Regression guard: EntryCategory must match design doc + lesson in diary.
#[tokio::test]
async fn test_entry_category_and_lesson_in_diary() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(true),
            path: Some(diary_path),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);
    let promoted = vec![
        PromotedGroupInfo {
            entity_name: "deploy".into(),
            entity_type: "subject".into(),
            lessons: vec!["verify before deploying".into()],
        },
        PromotedGroupInfo {
            entity_name: "user".into(),
            entity_type: "subject".into(),
            lessons: vec!["follow user style guide".into()],
        },
    ];
    pipeline.write_dream_diary(&promoted, None).await.unwrap();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let content = std::fs::read_to_string(tmp.path().join(format!("{}.md", date))).unwrap();
    assert!(content.contains("verify before deploying"));
    assert!(content.contains("follow user style guide"));
}

use closeclaw_config::agents::{
    DreamingCapacityConfig, DreamingScoringConfig, DreamingThresholdConfig,
};

// ── Deep stage: entity type weight + relative gate tests ─────────

/// Deep stage applies entity_type_weight dimension from SQLite entity_types table.
#[test]
fn test_deep_entity_type_weight_applied() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("etw.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE entity_types (type TEXT PRIMARY KEY, weight REAL NOT NULL, is_active INTEGER NOT NULL DEFAULT 1);
             INSERT INTO entity_types (type, weight, is_active) VALUES ('person', 2.0, 1);",
        )
        .unwrap();
    }
    let config = DreamingConfig {
        enabled: Some(true),
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            recency_weight: Some(1.0),
            explicitness_weight: Some(1.0),
            entity_type_weight_weight: Some(1.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
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
    let e1 = make_entry(EntryCategory::Decision, "about person", "s1", 1);
    let e2 = make_entry(EntryCategory::Decision, "about subject", "s1", 1);
    let groups = vec![
        EntityGroup {
            entity_name: "alice".into(),
            entity_type: "person".into(),
            entries: vec![e1],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
        EntityGroup {
            entity_name: "rust".into(),
            entity_type: "subject".into(),
            entries: vec![e2],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
    ];
    let result = pipeline.deep_stage(groups);
    let person = result.iter().find(|g| g.entity_name == "alice").unwrap();
    let subj = result.iter().find(|g| g.entity_name == "rust").unwrap();
    assert!(
        person.score > subj.score,
        "person {} > subject {}",
        person.score,
        subj.score
    );
}

/// Deep stage relative gate: per entity_type, removes groups below relative × top.
#[test]
fn test_deep_relative_gate_per_entity_type() {
    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.5),
        },
        capacity: DreamingCapacityConfig {
            max_rules: Some(100),
        },
        ..Default::default()
    });
    let e_high = {
        let mut e = make_entry(EntryCategory::Decision, "high", "s1", 1);
        e.entity_type = "subject".into();
        e.entity_name = "high".into();
        e
    };
    let e_low = {
        let mut e = make_entry(EntryCategory::Decision, "low", "s2", 1);
        e.entity_type = "subject".into();
        e.entity_name = "low".into();
        e
    };
    let e_person = {
        let mut e = make_entry(EntryCategory::Decision, "person", "s3", 1);
        e.entity_type = "person".into();
        e.entity_name = "p".into();
        e
    };
    let groups = vec![
        EntityGroup {
            entity_name: "high".into(),
            entity_type: "subject".into(),
            entries: vec![e_high],
            frequency: 10,
            cross_agent_count: 1,
            score: 0.0,
        },
        EntityGroup {
            entity_name: "low".into(),
            entity_type: "subject".into(),
            entries: vec![e_low],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
        EntityGroup {
            entity_name: "p".into(),
            entity_type: "person".into(),
            entries: vec![e_person],
            frequency: 3,
            cross_agent_count: 1,
            score: 0.0,
        },
    ];
    let result = pipeline.deep_stage(groups);
    assert!(
        !result.iter().any(|g| g.entity_name == "low"),
        "low should be filtered"
    );
    assert!(result.iter().any(|g| g.entity_name == "high"));
    assert!(result.iter().any(|g| g.entity_name == "p"));
}

// ── MEMORY.md write tests ──────────────────────────────────────────

/// MEMORY.md write creates file and parent directory.
#[test]
fn test_write_memory_md_creates_file_and_directory() {
    // Basic: creates file with rules.
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");
    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());
    let rules = vec!["always verify before deploying".to_string()];
    pipeline.write_memory_md(&rules).unwrap();
    let content = std::fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("- always verify before deploying"));
    // Nested path: auto-creates parent directory.
    let md_path2 = tmp.path().join("deep/nested/MEMORY.md");
    let pipeline2 = DreamingPipeline::new().with_memory_md_path(md_path2.to_str().unwrap());
    pipeline2.write_memory_md(&["rule".to_string()]).unwrap();
    assert!(md_path2.exists());
}

/// MEMORY.md write deduplicates existing rules.
#[test]
fn test_write_memory_md_deduplicates() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");
    // Pre-existing content with one rule.
    std::fs::write(&md_path, "- existing rule\n").unwrap();

    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());

    let rules = vec!["existing rule".to_string(), "new rule".to_string()];
    pipeline.write_memory_md(&rules).unwrap();

    let content = std::fs::read_to_string(&md_path).unwrap();
    // existing rule should appear only once.
    assert_eq!(content.matches("existing rule").count(), 1);
    assert!(content.contains("- new rule"));
}

/// MEMORY.md write appends without overwriting existing content.
#[test]
fn test_write_memory_md_appends() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");
    std::fs::write(&md_path, "- old rule\n").unwrap();

    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());

    pipeline
        .write_memory_md(&["added rule".to_string()])
        .unwrap();

    let content = std::fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("- old rule"));
    assert!(content.contains("- added rule"));
}

// ── LLM consolidation tests ────────────────────────────────────────

use crate::dreaming_llm::{DreamingLlmCaller, DreamingLlmError};
use async_trait::async_trait;
use std::sync::Arc;

/// Mock LLM caller that returns formatted rules.
struct MockConsolidationLlm;

#[async_trait]
impl DreamingLlmCaller for MockConsolidationLlm {
    async fn consolidate_lessons(
        &self,
        lessons: &[String],
        entity_name: &str,
        _entity_type: &str,
        _frequency: usize,
    ) -> Result<String, DreamingLlmError> {
        Ok(format!("[{}] {}", entity_name, lessons.join(", ")))
    }

    async fn generate_diary_narrative(
        &self,
        promoted_groups: &[PromotedGroupInfo],
    ) -> Result<String, DreamingLlmError> {
        let names: Vec<&str> = promoted_groups
            .iter()
            .map(|g| g.entity_name.as_str())
            .collect();
        Ok(format!("diary about {}", names.join(", ")))
    }
}

/// Failing LLM caller for testing degradation.
struct FailingConsolidationLlm;

#[async_trait]
impl DreamingLlmCaller for FailingConsolidationLlm {
    async fn consolidate_lessons(
        &self,
        _lessons: &[String],
        _entity_name: &str,
        _entity_type: &str,
        _frequency: usize,
    ) -> Result<String, DreamingLlmError> {
        Err(DreamingLlmError::Llm("simulated failure".into()))
    }

    async fn generate_diary_narrative(
        &self,
        _promoted_groups: &[PromotedGroupInfo],
    ) -> Result<String, DreamingLlmError> {
        Err(DreamingLlmError::Llm("simulated failure".into()))
    }
}

/// LLM consolidation produces rules from entity groups.
#[tokio::test]
async fn test_consolidate_lessons_produces_rules() {
    let pipeline = DreamingPipeline::new();
    let llm: Arc<dyn DreamingLlmCaller> = Arc::new(MockConsolidationLlm);

    let e1 = make_entry_with_lesson(
        EntryCategory::Error,
        "wrong deployment",
        "s1",
        "verify before deploy",
    );
    let e2 = make_entry(EntryCategory::Decision, "dark mode preferred", "s1", 5);

    let groups = vec![
        make_group(vec![e1], "deployment"),
        make_group(vec![e2], "ui"),
    ];
    let rules = pipeline.consolidate_lessons(&llm, &groups).await;
    assert_eq!(rules.len(), 2);
    assert!(rules[0].contains("deployment") || rules[1].contains("deployment"));
    assert!(rules[0].contains("ui") || rules[1].contains("ui"));
}

/// LLM failure falls back to raw lesson/body text.
#[tokio::test]
async fn test_consolidate_lessons_fallback_on_failure() {
    let pipeline = DreamingPipeline::new();
    let llm: Arc<dyn DreamingLlmCaller> = Arc::new(FailingConsolidationLlm);

    let groups = vec![make_group(
        vec![make_entry_with_lesson(
            EntryCategory::Error,
            "wrong deploy",
            "s1",
            "verify first",
        )],
        "x",
    )];

    let rules = pipeline.consolidate_lessons(&llm, &groups).await;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0], "verify first");
}

// ── SQLite integration tests ───────────────────────────────────────

/// SQLite integration: reads entries, handles missing DB/table.
#[tokio::test]
async fn test_collect_entries_sqlite_and_edge_cases() {
    // Setup: create DB with sessions + events + entities.
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, mined INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL,
             category TEXT NOT NULL, lesson TEXT, source_session_id TEXT NOT NULL,
             timestamp INTEGER NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE entities (id INTEGER PRIMARY KEY AUTOINCREMENT, agent_id TEXT NOT NULL,
             type TEXT NOT NULL, name TEXT NOT NULL, normalized_name TEXT NOT NULL,
             UNIQUE(agent_id, type, normalized_name));
             CREATE TABLE event_entities (id INTEGER PRIMARY KEY AUTOINCREMENT,
             event_id INTEGER NOT NULL, entity_id INTEGER NOT NULL);
             INSERT INTO sessions (id, mined) VALUES ('sess-1', 1);
             INSERT INTO events (content, category, lesson, source_session_id, timestamp, updated_at)
             VALUES ('test content', 'error', 'test lesson', 'sess-1', 1700000000, 1700000000);
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('agent-1', 'subject', 'Test Entity', 'test entity');
             INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1);",
        ).unwrap();
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
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config).with_db_path(&db_path);
    let entries = pipeline
        .collect_entries_for_session(&storage, "sess-1")
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].body, "test content");
    assert_eq!(entries[0].lesson.as_deref(), Some("test lesson"));
    assert_eq!(entries[0].category, EntryCategory::Error);
    assert!(entries[0].tags.contains(&"Test Entity".to_string()));
    assert_eq!(entries[0].event_id, 1);
    assert_eq!(entries[0].entity_type, "subject");
    assert_eq!(entries[0].entity_name, "Test Entity");
    assert_eq!(entries[0].updated_at, entries[0].timestamp);
    // No db_path → empty.
    let p2 = DreamingPipeline::new();
    let e2 = p2
        .collect_entries_for_session(&storage, "sess-1")
        .await
        .unwrap();
    assert!(e2.is_empty());
    // Missing table → empty, not error.
    let empty_db = tmp.path().join("empty.db");
    rusqlite::Connection::open(&empty_db).unwrap();
    let p3 = DreamingPipeline::new().with_db_path(&empty_db);
    let e3 = p3.collect_entries_for_session(&storage, "sess-1").await;
    assert!(e3.is_ok());
    assert!(e3.unwrap().is_empty());
    // Unminted session → entries filtered out.
    let unminted_db = tmp.path().join("unminted.db");
    {
        let conn = rusqlite::Connection::open(&unminted_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, mined INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL,
             category TEXT NOT NULL, lesson TEXT, source_session_id TEXT NOT NULL,
             timestamp INTEGER NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE entities (id INTEGER PRIMARY KEY AUTOINCREMENT, agent_id TEXT NOT NULL,
             type TEXT NOT NULL, name TEXT NOT NULL, normalized_name TEXT NOT NULL,
             UNIQUE(agent_id, type, normalized_name));
             CREATE TABLE event_entities (id INTEGER PRIMARY KEY AUTOINCREMENT,
             event_id INTEGER NOT NULL, entity_id INTEGER NOT NULL);
             INSERT INTO sessions (id, mined) VALUES ('sess-unminted', 0);
             INSERT INTO events (content, category, lesson, source_session_id, timestamp, updated_at)
             VALUES ('unminted content', 'error', NULL, 'sess-unminted', 1700000000, 1700000000);
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('agent-1', 'subject', 'Unminted Entity', 'unminted entity');
             INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1);",
        ).unwrap();
    }
    let mut cp_unminted = SessionCheckpoint::new("sess-unminted".into());
    cp_unminted.mined = false;
    storage.add_checkpoint(cp_unminted);
    let p4 = DreamingPipeline::new().with_db_path(&unminted_db);
    let e4 = p4
        .collect_entries_for_session(&storage, "sess-unminted")
        .await
        .unwrap();
    assert!(
        e4.is_empty(),
        "entries from unminted session should not be returned"
    );
}

// ── Config hot-reload tests ────────────────────────────────────────

/// update_config changes the enabled flag, and run_once stops skipping.
#[tokio::test]
async fn test_update_config_changes_behavior() {
    let storage = TestStorage::default();

    // Session mined + not yet dreamt.
    let mut cp = SessionCheckpoint::new("sess-reload".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    // Start with dreaming disabled.
    let config = DreamingConfig {
        enabled: Some(false),
        diary: DreamingDiaryConfig::default(),
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    // run_once should be a no-op (skips because disabled).
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once with disabled should succeed");

    // Verify session was NOT processed.
    {
        let cps = storage.checkpoints.lock().unwrap();
        let cp = cps.iter().find(|c| c.session_id == "sess-reload").unwrap();
        assert_eq!(
            cp.dreaming_status,
            DreamingStatus::Pending,
            "should still be Pending when disabled"
        );
    }

    // Hot-reload: enable dreaming.
    let new_config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig::default(),
        ..Default::default()
    };
    pipeline.update_config(new_config);

    // run_once should now process the session (enabled=true).
    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once after enable should succeed");

    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps.iter().find(|c| c.session_id == "sess-reload").unwrap();
    assert_eq!(
        cp.dreaming_status,
        DreamingStatus::Completed,
        "session should be Completed after hot-reload enable"
    );
}

// ── Anti-contamination tests ───────────────────────────────────────
/// Anti-contamination: event_id + updated_at check passes for valid events
/// and fails for stale events.
#[tokio::test]
async fn test_anti_contamination_valid_and_stale_events() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("ac.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, mined INTEGER NOT NULL DEFAULT 0);
             INSERT INTO sessions (id, mined) VALUES ('s1', 1);
             CREATE TABLE events (id INTEGER PRIMARY KEY, content TEXT, category TEXT,
             lesson TEXT, source_session_id TEXT, timestamp INTEGER, updated_at INTEGER);
             INSERT INTO events VALUES (42, 'test', 'error', NULL, 's1', 1000, 1000);",
        )
        .unwrap();
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let valid = MemoryEntry {
        category: EntryCategory::Error,
        body: "test".into(),
        timestamp: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        source_session_id: "s1".into(),
        lesson: None,
        tags: vec![],
        score: 0.0,
        event_id: 42,
        entity_type: "subject".into(),
        entity_name: "x".into(),
        updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
    };
    assert!(pipeline.verify_event_integrity(&conn, &valid).unwrap());
    let stale = MemoryEntry {
        updated_at: chrono::DateTime::from_timestamp(2000, 0).unwrap(),
        ..valid.clone()
    };
    assert!(!pipeline.verify_event_integrity(&conn, &stale).unwrap());
}

/// Anti-contamination: verify_and_filter_rules skips rules with stale events.
#[tokio::test]
async fn test_verify_and_filter_rules_drops_stale() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("ac3.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, mined INTEGER NOT NULL DEFAULT 0);
             INSERT INTO sessions (id, mined) VALUES ('s1', 1);
             CREATE TABLE events (id INTEGER PRIMARY KEY, content TEXT, category TEXT,
             lesson TEXT, source_session_id TEXT, timestamp INTEGER, updated_at INTEGER);
             INSERT INTO events VALUES (1, 'a', 'error', NULL, 's1', 1000, 1000);
             INSERT INTO events VALUES (2, 'b', 'error', NULL, 's1', 1000, 2000);",
        )
        .unwrap();
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let groups = vec![
        EntityGroup {
            entity_name: "a".into(),
            entity_type: "subject".into(),
            entries: vec![MemoryEntry {
                category: EntryCategory::Error,
                body: "a".into(),
                timestamp: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
                source_session_id: "s1".into(),
                lesson: None,
                tags: vec![],
                score: 0.0,
                event_id: 1,
                entity_type: "subject".into(),
                entity_name: "a".into(),
                updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
            }],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
        EntityGroup {
            entity_name: "b".into(),
            entity_type: "subject".into(),
            entries: vec![MemoryEntry {
                category: EntryCategory::Error,
                body: "b".into(),
                timestamp: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
                source_session_id: "s1".into(),
                lesson: None,
                tags: vec![],
                score: 0.0,
                event_id: 2,
                entity_type: "subject".into(),
                entity_name: "b".into(),
                updated_at: chrono::DateTime::from_timestamp(9999, 0).unwrap(),
            }],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
    ];
    let rules = vec!["rule a".into(), "rule b".into()];
    let verified = pipeline
        .verify_and_filter_rules(&rules, &groups)
        .await
        .unwrap();
    assert_eq!(verified.len(), 1);
    assert_eq!(verified[0], "rule a");
}
// ── DreamingPipeline model propagation tests ───────────────────────
/// Model extraction, default None, and lifecycle via update_config.
#[test]
fn test_model_lifecycle() {
    let config = DreamingConfig {
        model: Some("gpt-4o".to_string()),
        ..Default::default()
    };
    let p = DreamingPipeline::with_config(config);
    assert_eq!(p.model().as_deref(), Some("gpt-4o"));
    assert_eq!(DreamingPipeline::default().model(), None);
    p.update_config(DreamingConfig {
        model: Some("claude-3.5-sonnet".to_string()),
        ..Default::default()
    });
    assert_eq!(p.model().as_deref(), Some("claude-3.5-sonnet"));
    p.update_config(DreamingConfig {
        model: None,
        ..Default::default()
    });
    assert_eq!(p.model(), None);
}

// ── REM stage: cross-agent detection ─────────────────────────────
/// REM stage detects cross-agent entity sharing via SQLite agent map.
#[test]
fn test_rem_cross_agent_detection() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("rem_ca.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE entities (id INTEGER PRIMARY KEY, agent_id TEXT, type TEXT,
             name TEXT, normalized_name TEXT, UNIQUE(agent_id, type, normalized_name));
             INSERT INTO entities VALUES (1, 'agent-a', 'subject', 'rust', 'rust');
             INSERT INTO entities VALUES (2, 'agent-b', 'subject', 'rust', 'rust');
             INSERT INTO entities VALUES (3, 'agent-a', 'subject', 'python', 'python');",
        )
        .unwrap();
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let e1 = {
        let mut e = make_entry(EntryCategory::Error, "err1", "s1", 10);
        e.entity_name = "rust".into();
        e.entity_type = "subject".into();
        e
    };
    let e2 = {
        let mut e = make_entry(EntryCategory::Error, "err2", "s2", 10);
        e.entity_name = "rust".into();
        e.entity_type = "subject".into();
        e
    };
    let e3 = {
        let mut e = make_entry(EntryCategory::Error, "err3", "s1", 10);
        e.entity_name = "python".into();
        e.entity_type = "subject".into();
        e
    };
    let groups = pipeline.rem_stage(vec![vec![e1, e2], vec![e3]]);
    let rust = groups.iter().find(|g| g.entity_name == "rust").unwrap();
    assert_eq!(
        rust.cross_agent_count, 2,
        "rust shared by agent-a and agent-b"
    );
    let python = groups.iter().find(|g| g.entity_name == "python").unwrap();
    assert_eq!(python.cross_agent_count, 1, "python only by agent-a");
}

// ── End-to-end pipeline test ─────────────────────────────────────
/// End-to-end: Light → REM → Deep pipeline flow with entity grouping.
#[test]
fn test_e2e_light_rem_deep_pipeline() {
    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            recency_weight: Some(0.5),
            explicitness_weight: Some(1.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
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
    });
    let mut e1 = make_entry(EntryCategory::Decision, "deploy fast", "s1", 1);
    e1.entity_type = "subject".into();
    e1.entity_name = "deploy".into();
    let mut e2 = make_entry(EntryCategory::Decision, "deploy careful", "s2", 1);
    e2.entity_type = "subject".into();
    e2.entity_name = "deploy".into();
    let mut e3 = make_entry(EntryCategory::Error, "vim error", "s1", 1);
    e3.entity_type = "subject".into();
    e3.entity_name = "vim".into();
    // Light: dedup + entity type chunking
    let light = pipeline.light_stage(vec![e1, e2, e3]).unwrap();
    assert_eq!(light.len(), 1, "all same entity_type -> 1 chunk");
    // REM: cluster by entity
    let rem = pipeline.rem_stage(light);
    assert_eq!(rem.len(), 2, "two distinct entities");
    let deploy = rem.iter().find(|g| g.entity_name == "deploy").unwrap();
    assert_eq!(deploy.frequency, 2, "deploy appears in 2 sessions");
    // Deep: score and filter
    let deep = pipeline.deep_stage(rem);
    assert!(!deep.is_empty(), "at least one group should survive");
    for g in &deep {
        assert!(g.score >= 0.0, "score should be non-negative");
    }
}
// ── Light stage entity-type chunking + semantic dedup tests ────────
/// Semantic dedup: filters overlapping, keeps non-overlapping entries.
#[test]
fn test_light_dedup_semantic() {
    let pipeline = DreamingPipeline::new();
    // Overlapping entry should be filtered.
    let entries = vec![
        make_entry(
            EntryCategory::Decision,
            "always prefer dark mode theme",
            "s1",
            10,
        ),
        make_entry(EntryCategory::Decision, "use vim for code editing", "s1", 5),
    ];
    let existing = vec!["prefer dark mode theme always".to_string()];
    let result = pipeline.deduplicate(entries, &existing);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].body, "use vim for code editing");
    // Non-overlapping: all entries kept.
    let entries2 = vec![
        make_entry(EntryCategory::Decision, "dark mode preferred", "s1", 10),
        make_entry(EntryCategory::Error, "deployment failed", "s2", 5),
    ];
    let result2 = pipeline.deduplicate(entries2, &["unrelated rule about testing".to_string()]);
    assert_eq!(result2.len(), 2);
}

/// Entity-type chunking groups entries by entity_type field.
#[test]
fn test_light_chunk_by_entity_type() {
    let pipeline = DreamingPipeline::new();
    let mut e1 = make_entry(EntryCategory::Decision, "a", "s1", 10);
    e1.entity_type = "subject".to_string();
    let mut e2 = make_entry(EntryCategory::Decision, "b", "s2", 10);
    e2.entity_type = "person".to_string();
    let mut e3 = make_entry(EntryCategory::Decision, "c", "s1", 5);
    e3.entity_type = "subject".to_string();
    let chunks = pipeline.chunk_by_entity_type(vec![e1, e2, e3]);
    assert_eq!(chunks.len(), 2);
    let subject: Vec<_> = chunks
        .iter()
        .filter(|c| c[0].entity_type == "subject")
        .collect();
    assert_eq!(subject.len(), 1);
    assert_eq!(subject[0].len(), 2);
}
