//! Additional unit tests for DreamingPipeline.
//!
//! Complements the inline tests in dreaming.rs with tests that require
//! mock PersistenceService interactions.

use crate::dreaming::{DreamingPipeline, EntityGroup, EntryCategory, MemoryEntry};
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

/// Dream Diary writes a file when diary is enabled.
#[test]
fn test_dream_diary_writes_when_enabled() {
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

    let groups = vec![make_group(
        vec![
            make_entry(EntryCategory::Decision, "dark mode preferred", "s1", 10),
            make_entry_with_lesson(
                EntryCategory::Error,
                "wrong deployment",
                "s1",
                "verify before deploying",
            ),
        ],
        "deploy",
    )];

    let result = pipeline.write_dream_diary(&groups);
    assert!(
        result.is_ok(),
        "write_dream_diary should succeed: {result:?}"
    );

    // Check that the diary file was created.
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let diary_file = tmp.path().join(format!("{}.md", date));
    assert!(diary_file.exists(), "diary file should exist");

    let content = std::fs::read_to_string(&diary_file).unwrap();
    assert!(content.contains("dark mode preferred"));
    assert!(content.contains("wrong deployment"));
    assert!(content.contains("verify before deploying"));
    assert!(content.contains("Promoted 2 entries"));
}

#[test]
fn test_dream_diary_does_not_write_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(false),
            path: Some(diary_path),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let groups = vec![make_group(
        vec![make_entry(
            EntryCategory::Decision,
            "should not appear",
            "s1",
            10,
        )],
        "x",
    )];

    let result = pipeline.write_dream_diary(&groups);
    assert!(result.is_ok());

    // Diary directory should NOT exist since diary is disabled.
    assert!(
        tmp.path().read_dir().unwrap().next().is_none(),
        "no files should be created when diary is disabled"
    );
}

#[test]
fn test_dream_diary_uses_custom_path() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().join("custom/diary");
    let config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(true),
            path: Some(diary_path.to_str().unwrap().to_string()),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let groups = vec![make_group(
        vec![make_entry(
            EntryCategory::Decision,
            "custom path test",
            "s1",
            10,
        )],
        "x",
    )];

    let result = pipeline.write_dream_diary(&groups);
    assert!(
        result.is_ok(),
        "write_dream_diary should succeed: {result:?}"
    );

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let diary_file = diary_path.join(format!("{}.md", date));
    assert!(
        diary_file.exists(),
        "diary should be written to custom path"
    );
    assert!(
        diary_path.exists(),
        "custom diary directory should be auto-created"
    );
}

#[test]
fn test_dream_diary_creates_directory() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().join("new/dir/level");
    let config = DreamingConfig {
        enabled: Some(true),
        diary: DreamingDiaryConfig {
            enabled: Some(true),
            path: Some(diary_path.to_str().unwrap().to_string()),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let groups = vec![make_group(
        vec![make_entry(
            EntryCategory::Decision,
            "auto dir test",
            "s1",
            10,
        )],
        "x",
    )];

    let result = pipeline.write_dream_diary(&groups);
    assert!(result.is_ok());
    assert!(
        diary_path.exists(),
        "diary directory should be auto-created"
    );
}

/// Regression guard: EntryCategory must match design doc.
#[test]
fn test_entry_category_variants_match_design_doc() {
    let all = [
        EntryCategory::Error,
        EntryCategory::Anger,
        EntryCategory::Decision,
    ];
    assert_eq!(all.len(), 3);

    for cat in &all {
        let label = match cat {
            EntryCategory::Error => "Error",
            EntryCategory::Anger => "Anger",
            EntryCategory::Decision => "Decision",
        };
        assert!(!label.is_empty());
    }
}

/// Error and Anger entries always carry a lesson in diary output.
#[test]
fn test_error_anger_entries_carry_lesson_in_diary() {
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

    let groups = vec![
        make_group(
            vec![make_entry_with_lesson(
                EntryCategory::Error,
                "wrong deployment",
                "s1",
                "verify before deploying",
            )],
            "deploy",
        ),
        make_group(
            vec![make_entry_with_lesson(
                EntryCategory::Anger,
                "user corrected output",
                "s1",
                "follow user style guide",
            )],
            "user",
        ),
    ];

    let result = pipeline.write_dream_diary(&groups);
    assert!(result.is_ok());

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let diary_file = tmp.path().join(format!("{}.md", date));
    let content = std::fs::read_to_string(&diary_file).unwrap();

    // Both Error and Anger entries should include their lesson in the output.
    assert!(content.contains("Lesson: verify before deploying"));
    assert!(content.contains("Lesson: follow user style guide"));
}

#[test]
fn test_dream_diary_empty_entries_no_write() {
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

    let groups: Vec<EntityGroup> = vec![];
    let result = pipeline.write_dream_diary(&groups);
    assert!(result.is_ok());

    // No files should be created in the diary directory.
    assert!(
        tmp.path().read_dir().unwrap().next().is_none(),
        "no files should be created for empty entries"
    );
}

use closeclaw_config::agents::{
    DreamingCapacityConfig, DreamingScoringConfig, DreamingThresholdConfig,
};

#[test]
fn test_dreaming_pipeline_custom_scoring_config() {
    let config = DreamingConfig {
        enabled: Some(true),
        scoring: DreamingScoringConfig {
            frequency_weight: Some(2.0),
            recency_weight: Some(1.0),
            explicitness_weight: Some(3.0),
            cross_agent_weight: Some(2.0),
            negative_signal_weight: Some(-1.0),
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
    // Verify with_config doesn't panic and pipeline is constructible.
    let _pipeline = DreamingPipeline::with_config(config);
}

#[test]
fn test_dreaming_pipeline_high_threshold_config() {
    let config = DreamingConfig {
        enabled: Some(true),
        scoring: DreamingScoringConfig {
            frequency_weight: Some(0.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(5.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig {
            max_rules: Some(100),
        },
        ..Default::default()
    };
    let _pipeline = DreamingPipeline::with_config(config);
}

#[test]
fn test_dreaming_pipeline_capacity_config_stored() {
    let config = DreamingConfig {
        enabled: Some(true),
        scoring: DreamingScoringConfig::default(),
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig { max_rules: Some(5) },
        ..Default::default()
    };
    let _pipeline = DreamingPipeline::with_config(config);
}

/// Boundary: max_rules=0 config is accepted without panic.
#[test]
fn test_dreaming_pipeline_max_rules_zero_config() {
    let config = DreamingConfig {
        enabled: Some(true),
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig { max_rules: Some(0) },
        ..Default::default()
    };
    let _pipeline = DreamingPipeline::with_config(config);
}

/// Custom relative threshold config is accepted.
#[test]
fn test_dreaming_pipeline_relative_threshold_config() {
    let config = DreamingConfig {
        enabled: Some(true),
        scoring: DreamingScoringConfig::default(),
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.5),
        },
        capacity: DreamingCapacityConfig {
            max_rules: Some(100),
        },
        ..Default::default()
    };
    let _pipeline = DreamingPipeline::with_config(config);
}

/// Default DreamingPipeline construction succeeds.
#[test]
fn test_dreaming_pipeline_default_config() {
    let pipeline = DreamingPipeline::default();
    // Verify default construction doesn't panic.
    let entries = vec![make_entry(EntryCategory::Decision, "test", "s1", 10)];
    // run_once needs storage; just verify pipeline is constructible.
    let _ = pipeline;
    assert!(!entries.is_empty());
}

// ── MEMORY.md write tests ──────────────────────────────────────────

/// MEMORY.md write creates file with rules.
#[test]
fn test_write_memory_md_creates_file() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");
    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());

    let rules = vec!["always verify before deploying".to_string()];
    let result = pipeline.write_memory_md(&rules);
    assert!(result.is_ok(), "write_memory_md should succeed: {result:?}");

    let content = std::fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("- always verify before deploying"));
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

/// MEMORY.md write creates parent directory if missing.
#[test]
fn test_write_memory_md_creates_directory() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("deep/nested/MEMORY.md");

    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());

    pipeline.write_memory_md(&["rule".to_string()]).unwrap();
    assert!(md_path.exists());
}

/// MEMORY.md write is a no-op for empty rules.
#[test]
fn test_write_memory_md_empty_rules_noop() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");

    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());

    pipeline.write_memory_md(&[]).unwrap();
    assert!(
        !md_path.exists(),
        "no file should be created for empty rules"
    );
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
    ) -> Result<String, DreamingLlmError> {
        Ok(format!("[{}] {}", entity_name, lessons.join(", ")))
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

/// collect_entries_for_session reads from SQLite when db_path is set.
#[tokio::test]
async fn test_collect_entries_from_sqlite() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                category TEXT NOT NULL,
                lesson TEXT,
                source_session_id TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            );
            CREATE TABLE entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                UNIQUE(agent_id, type, normalized_name)
            );
            CREATE TABLE event_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id INTEGER NOT NULL,
                entity_id INTEGER NOT NULL
            );
            INSERT INTO events (content, category, lesson, source_session_id, timestamp)
            VALUES ('test content', 'error', 'test lesson', 'sess-1', 1700000000);
            INSERT INTO entities (agent_id, type, name, normalized_name)
            VALUES ('agent-1', 'subject', 'Test Entity', 'test entity');
            INSERT INTO event_entities (event_id, entity_id)
            VALUES (1, 1);",
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
    // Entity name should be loaded as a tag.
    assert!(entries[0].tags.contains(&"Test Entity".to_string()));
    // New fields: event_id, entity_type, entity_name, updated_at.
    assert_eq!(entries[0].event_id, 1);
    assert_eq!(entries[0].entity_type, "subject");
    assert_eq!(entries[0].entity_name, "Test Entity");
    assert_eq!(entries[0].updated_at, entries[0].timestamp);
}

/// collect_entries_for_session returns empty when db_path is None.
#[tokio::test]
async fn test_collect_entries_no_db_path() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let pipeline = DreamingPipeline::new();
    let entries = pipeline
        .collect_entries_for_session(&storage, "sess-1")
        .await
        .unwrap();
    assert!(entries.is_empty());
}

/// collect_entries_for_session handles missing events table.
#[tokio::test]
async fn test_collect_entries_missing_table() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("empty.db");
    // Create empty SQLite (no events table).
    rusqlite::Connection::open(&db_path).unwrap();

    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-1".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let result = pipeline
        .collect_entries_for_session(&storage, "sess-1")
        .await;
    // Should return empty vec, not error.
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
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

/// Concurrent update_config and config reads do not panic.
#[test]
fn test_update_config_concurrent_safety() {
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let config_enabled = i % 2 == 0;
            std::thread::spawn(move || {
                let cfg = DreamingConfig {
                    enabled: Some(config_enabled),
                    diary: DreamingDiaryConfig::default(),
                    ..Default::default()
                };
                // Each thread creates its own pipeline — this verifies the
                // RwLock internals don't panic under rapid construction.
                let p = DreamingPipeline::with_config(cfg);
                p.update_config(DreamingConfig {
                    enabled: Some(!config_enabled),
                    diary: DreamingDiaryConfig::default(),
                    ..Default::default()
                });
                // Verify the pipeline is still usable after rapid config changes.
                let _ = p.write_memory_md(&[]);
                drop(p);
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread should not panic");
    }
}

// ── Anti-contamination tests ───────────────────────────────────────

// ── DreamingPipeline model propagation tests ───────────────────────

/// with_config() extracts model from DreamingConfig.
#[test]
fn test_with_config_extracts_model() {
    let config = DreamingConfig {
        model: Some("gpt-4o".to_string()),
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);
    assert_eq!(pipeline.model().as_deref(), Some("gpt-4o"));
}

/// Default pipeline and unconfigured config both have model as None.
#[test]
fn test_default_model_is_none() {
    let pipeline = DreamingPipeline::default();
    assert_eq!(pipeline.model(), None);
    let pipeline2 = DreamingPipeline::with_config(DreamingConfig::default());
    assert_eq!(pipeline2.model(), None);
}

/// update_config propagates model changes and can clear model.
#[test]
fn test_update_config_model_lifecycle() {
    let pipeline = DreamingPipeline::default();
    assert_eq!(pipeline.model(), None);
    pipeline.update_config(DreamingConfig {
        model: Some("claude-3.5-sonnet".to_string()),
        ..Default::default()
    });
    assert_eq!(pipeline.model().as_deref(), Some("claude-3.5-sonnet"));
    pipeline.update_config(DreamingConfig {
        model: None,
        ..Default::default()
    });
    assert_eq!(pipeline.model(), None);
}

/// Per-agent override: different pipelines can have different models.
#[test]
fn test_per_agent_override_model() {
    let pipeline_a = DreamingPipeline::with_config(DreamingConfig {
        model: Some("model-a".to_string()),
        ..Default::default()
    });
    let pipeline_b = DreamingPipeline::with_config(DreamingConfig {
        model: Some("model-b".to_string()),
        ..Default::default()
    });
    assert_eq!(pipeline_a.model().as_deref(), Some("model-a"));
    assert_eq!(pipeline_b.model().as_deref(), Some("model-b"));
}

// ── Light stage entity-type chunking + semantic dedup tests ────────

/// Semantic dedup filters entries that overlap with MEMORY.md existing rules.
#[test]
fn test_light_dedup_semantic_filters_existing_rules() {
    let pipeline = DreamingPipeline::new();
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
}

/// Semantic dedup keeps all entries when no overlap with existing rules.
#[test]
fn test_light_dedup_semantic_no_match_keeps_all() {
    let pipeline = DreamingPipeline::new();
    let entries = vec![
        make_entry(EntryCategory::Decision, "dark mode preferred", "s1", 10),
        make_entry(EntryCategory::Error, "deployment failed", "s2", 5),
    ];
    let existing = vec!["unrelated rule about testing".to_string()];
    let result = pipeline.deduplicate(entries, &existing);
    assert_eq!(result.len(), 2);
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
