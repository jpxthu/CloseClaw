//! Tests for the 3 gap-fixes: anti-contamination updated_at read,
//! lesson consolidation entity_type/frequency, and capacity limit.

use crate::dreaming::{DreamingPipeline, EntityGroup, EntryCategory, MemoryEntry};
use crate::dreaming_llm::{DreamingLlmCaller, DreamingLlmError, PromotedGroupInfo};
use closeclaw_config::agents::{
    DreamingCapacityConfig, DreamingConfig, DreamingScoringConfig, DreamingThresholdConfig,
};
use tempfile::TempDir;

// ── Helpers ─────────────────────────────────────────────────────────

fn make_entry_with_timestamps(
    category: EntryCategory,
    body: &str,
    session_id: &str,
    event_id: i64,
    timestamp: i64,
    updated_at: i64,
    entity_type: &str,
    entity_name: &str,
) -> MemoryEntry {
    MemoryEntry {
        category,
        body: body.to_string(),
        timestamp: chrono::DateTime::from_timestamp(timestamp, 0).unwrap(),
        source_session_id: session_id.to_string(),
        lesson: None,
        tags: Vec::new(),
        score: 0.0,
        event_id,
        entity_type: entity_type.to_string(),
        entity_name: entity_name.to_string(),
        updated_at: chrono::DateTime::from_timestamp(updated_at, 0).unwrap(),
    }
}

// ── Anti-contamination: updated_at ≠ timestamp ────────────────────

/// When DB updated_at differs from timestamp, load_entries_from_sqlite
/// reads the correct updated_at value (not timestamp).
#[test]
fn test_anti_contamination_updated_at_from_db() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("ac_fix.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL, category TEXT NOT NULL,
                lesson TEXT, source_session_id TEXT NOT NULL,
                timestamp INTEGER NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL, type TEXT NOT NULL,
                name TEXT NOT NULL, normalized_name TEXT NOT NULL,
                UNIQUE(agent_id, type, normalized_name));
             CREATE TABLE event_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id INTEGER NOT NULL, entity_id INTEGER NOT NULL);
             INSERT INTO events (content, category, lesson, source_session_id,
                timestamp, updated_at)
             VALUES ('content', 'error', 'lesson1', 'sess-1', 1000, 5000);
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('a1', 'subject', 'EntA', 'enta');
             INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1);",
        )
        .unwrap();
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let entries = pipeline.load_entries_from_sqlite(&conn, "sess-1").unwrap();

    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(
        e.timestamp,
        chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        "timestamp should be 1000"
    );
    assert_eq!(
        e.updated_at,
        chrono::DateTime::from_timestamp(5000, 0).unwrap(),
        "updated_at should be 5000 (from DB), not 1000 (timestamp)"
    );
}

/// verify_event_integrity passes when entry.updated_at matches DB updated_at
/// even though they differ from timestamp.
#[tokio::test]
async fn test_anti_contamination_integrity_uses_updated_at_not_timestamp() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("ac_fix2.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (
                id INTEGER PRIMARY KEY, content TEXT, category TEXT,
                lesson TEXT, source_session_id TEXT,
                timestamp INTEGER, updated_at INTEGER);
             INSERT INTO events VALUES (1, 'test', 'error', NULL, 's1', 1000, 5000);",
        )
        .unwrap();
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Entry with correct updated_at (5000) but different timestamp (1000).
    let entry = make_entry_with_timestamps(
        EntryCategory::Error,
        "test",
        "s1",
        1,
        1000, // timestamp
        5000, // updated_at = DB updated_at
        "subject",
        "x",
    );
    assert!(
        pipeline.verify_event_integrity(&conn, &entry).unwrap(),
        "should pass: updated_at matches DB value"
    );

    // Entry with stale updated_at (3000) even though timestamp matches DB timestamp.
    let stale_entry = make_entry_with_timestamps(
        EntryCategory::Error,
        "test",
        "s1",
        1,
        1000, // timestamp matches DB timestamp
        3000, // updated_at does NOT match DB updated_at (5000)
        "subject",
        "x",
    );
    assert!(
        !pipeline
            .verify_event_integrity(&conn, &stale_entry)
            .unwrap(),
        "should fail: updated_at does not match DB value"
    );
}

// ── Lesson consolidation: entity_type & frequency ──────────────────

/// Mock LLM that records the entity_type and frequency it received.
struct ParamRecordingLlm {
    recorded_params: std::sync::Mutex<Vec<(String, usize)>>,
}

impl ParamRecordingLlm {
    fn new() -> Self {
        Self {
            recorded_params: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<(String, usize)> {
        self.recorded_params.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl DreamingLlmCaller for ParamRecordingLlm {
    async fn consolidate_lessons(
        &self,
        lessons: &[String],
        _entity_name: &str,
        entity_type: &str,
        frequency: usize,
    ) -> Result<String, DreamingLlmError> {
        self.recorded_params
            .lock()
            .unwrap()
            .push((entity_type.to_string(), frequency));
        if lessons.is_empty() {
            return Err(DreamingLlmError::Llm("no lessons".into()));
        }
        Ok(format!("rule: {}", lessons.join(", ")))
    }

    async fn generate_diary_narrative(
        &self,
        _promoted_groups: &[PromotedGroupInfo],
    ) -> Result<String, DreamingLlmError> {
        Ok("diary".into())
    }
}

/// consolidate_lessons passes entity_type and frequency to the LLM caller.
#[tokio::test]
async fn test_consolidate_lessons_passes_entity_type_and_frequency() {
    let pipeline = DreamingPipeline::new();
    let mock = std::sync::Arc::new(ParamRecordingLlm::new());
    let llm: std::sync::Arc<dyn DreamingLlmCaller> = mock.clone();

    let mut e1 = make_entry_with_timestamps(
        EntryCategory::Error,
        "err1",
        "s1",
        1,
        1000,
        1000,
        "person",
        "alice",
    );
    e1.lesson = Some("always greet alice".into());
    let mut e2 = make_entry_with_timestamps(
        EntryCategory::Error,
        "err2",
        "s2",
        2,
        2000,
        2000,
        "person",
        "alice",
    );
    e2.lesson = Some("remember alice birthday".into());

    let group = EntityGroup {
        entity_name: "alice".into(),
        entity_type: "person".into(),
        entries: vec![e1, e2],
        frequency: 3,
        cross_agent_count: 1,
        score: 0.0,
    };

    let rules = pipeline.consolidate_lessons(&llm, &[group]).await;
    assert_eq!(rules.len(), 1);

    let recorded = mock.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "person", "entity_type should be 'person'");
    assert_eq!(recorded[0].1, 3, "frequency should be 3");
}

/// Multiple groups: each passes its own entity_type and frequency.
#[tokio::test]
async fn test_consolidate_lessons_multiple_groups_distinct_params() {
    let pipeline = DreamingPipeline::new();
    let mock = std::sync::Arc::new(ParamRecordingLlm::new());
    let llm: std::sync::Arc<dyn DreamingLlmCaller> = mock.clone();

    let mut e1 = make_entry_with_timestamps(
        EntryCategory::Decision,
        "dec1",
        "s1",
        1,
        1000,
        1000,
        "subject",
        "rust",
    );
    e1.lesson = Some("use rust".into());

    let mut e2 = make_entry_with_timestamps(
        EntryCategory::Error,
        "err1",
        "s1",
        2,
        2000,
        2000,
        "person",
        "bob",
    );
    e2.lesson = Some("greet bob".into());

    let groups = vec![
        EntityGroup {
            entity_name: "rust".into(),
            entity_type: "subject".into(),
            entries: vec![e1],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
        EntityGroup {
            entity_name: "bob".into(),
            entity_type: "person".into(),
            entries: vec![e2],
            frequency: 5,
            cross_agent_count: 2,
            score: 0.0,
        },
    ];

    let rules = pipeline.consolidate_lessons(&llm, &groups).await;
    assert_eq!(rules.len(), 2);

    let recorded = mock.recorded();
    assert_eq!(recorded.len(), 2);

    // Find the entry for each entity.
    let rust_params = recorded.iter().find(|(t, _)| t == "subject");
    let bob_params = recorded.iter().find(|(t, _)| t == "person");

    assert!(rust_params.is_some(), "should have subject entity_type");
    assert_eq!(rust_params.unwrap().1, 1, "rust frequency should be 1");

    assert!(bob_params.is_some(), "should have person entity_type");
    assert_eq!(bob_params.unwrap().1, 5, "bob frequency should be 5");
}

// ── Capacity limit: Gate 3 respects existing MEMORY.md ─────────────

/// Gate 3 limits new entries by remaining capacity (max_rules - existing).
#[test]
fn test_deep_capacity_limit_considers_existing_rules() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");
    // Pre-populate MEMORY.md with 2 rules.
    std::fs::write(&md_path, "- rule1\n- rule2\n").unwrap();

    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(10.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig { max_rules: Some(5) },
        ..Default::default()
    })
    .with_memory_md_path(md_path.to_str().unwrap());

    // Create 4 high-scoring groups (all pass absolute/relative gates).
    let groups: Vec<EntityGroup> = (0..4)
        .map(|i| {
            let mut e = make_entry_with_timestamps(
                EntryCategory::Decision,
                &format!("rule body {}", i),
                &format!("s{}", i),
                (i + 1) as i64,
                1000 + i as i64,
                1000 + i as i64,
                "subject",
                &format!("entity{}", i),
            );
            e.lesson = Some(format!("lesson {}", i));
            EntityGroup {
                entity_name: format!("entity{}", i),
                entity_type: "subject".into(),
                entries: vec![e],
                frequency: 10,
                cross_agent_count: 1,
                score: 0.0,
            }
        })
        .collect();

    let deep = pipeline.deep_stage(groups);

    // max_rules=5, existing=2, remaining=3 → only 3 should pass Gate 3.
    assert!(
        deep.len() <= 3,
        "should allow at most 3 new entries (5 - 2), got {}",
        deep.len()
    );
}

/// Gate 3 drops all entries when MEMORY.md is already at capacity.
#[test]
fn test_deep_capacity_limit_full_drops_all() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");
    // Pre-populate MEMORY.md with 5 rules (at capacity).
    let content: String = (0..5).map(|i| format!("- rule{}\n", i)).collect();
    std::fs::write(&md_path, &content).unwrap();

    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(10.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig { max_rules: Some(5) },
        ..Default::default()
    })
    .with_memory_md_path(md_path.to_str().unwrap());

    let groups: Vec<EntityGroup> = (0..3)
        .map(|i| {
            let mut e = make_entry_with_timestamps(
                EntryCategory::Decision,
                &format!("new rule {}", i),
                &format!("s{}", i),
                (i + 1) as i64,
                1000,
                1000,
                "subject",
                &format!("entity{}", i),
            );
            e.lesson = Some(format!("new lesson {}", i));
            EntityGroup {
                entity_name: format!("entity{}", i),
                entity_type: "subject".into(),
                entries: vec![e],
                frequency: 10,
                cross_agent_count: 1,
                score: 0.0,
            }
        })
        .collect();

    let deep = pipeline.deep_stage(groups);

    // existing=5, max_rules=5, remaining=0 → all dropped.
    assert!(
        deep.is_empty(),
        "all entries should be dropped when at capacity, got {}",
        deep.len()
    );
}

/// Gate 3 with no MEMORY.md file: full capacity available.
#[test]
fn test_deep_capacity_limit_no_existing_file() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("MEMORY.md");
    // No file → existing_count = 0.

    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(10.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(0.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(0.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig { max_rules: Some(3) },
        ..Default::default()
    })
    .with_memory_md_path(md_path.to_str().unwrap());

    let groups: Vec<EntityGroup> = (0..3)
        .map(|i| {
            let mut e = make_entry_with_timestamps(
                EntryCategory::Decision,
                &format!("rule {}", i),
                &format!("s{}", i),
                (i + 1) as i64,
                1000,
                1000,
                "subject",
                &format!("entity{}", i),
            );
            e.lesson = Some(format!("lesson {}", i));
            EntityGroup {
                entity_name: format!("entity{}", i),
                entity_type: "subject".into(),
                entries: vec![e],
                frequency: 10,
                cross_agent_count: 1,
                score: 0.0,
            }
        })
        .collect();

    let deep = pipeline.deep_stage(groups);

    // existing=0, max_rules=3 → all 3 should pass.
    assert_eq!(deep.len(), 3, "all 3 should pass with no existing rules");
}
