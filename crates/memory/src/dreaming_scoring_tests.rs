//! Tests for negative_signal scoring, Dream Diary LLM narrative,
//! and run_once integration.

use crate::dreaming::{DreamingPipeline, EntityGroup, EntryCategory, MemoryEntry};
use crate::dreaming_llm::{DreamingLlmCaller, DreamingLlmError, PromotedGroupInfo};
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::{
    DreamingCapacityConfig, DreamingConfig, DreamingDiaryConfig, DreamingScoringConfig,
    DreamingThresholdConfig,
};
use closeclaw_session::persistence::{DreamingStatus, SessionCheckpoint};
use tempfile::TempDir;

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

// ── negative_signal scoring tests ────────────────────────────────

/// Same category entries → negative_signal = 0.0, score unaffected.
#[test]
fn test_negative_signal_same_category() {
    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
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
    });
    let mut e1 = make_entry(EntryCategory::Error, "err1", "s1", 10);
    e1.entity_type = "subject".into();
    e1.entity_name = "deploy".into();
    let mut e2 = make_entry(EntryCategory::Error, "err2", "s1", 5);
    e2.entity_type = "subject".into();
    e2.entity_name = "deploy".into();
    let groups = vec![EntityGroup {
        entity_name: "deploy".into(),
        entity_type: "subject".into(),
        entries: vec![e1, e2],
        frequency: 1,
        cross_agent_count: 1,
        score: 0.0,
    }];
    let deep = pipeline.deep_stage(groups);
    let g = &deep[0];
    // All same category → negative_signal = 0.0
    // score = frequency * 1.0 = 1.0 (no negative_signal penalty)
    assert!(g.score > 0.0, "score should be positive: {}", g.score);
}

/// Mixed category entries → negative_signal > 0.0, score reduced.
#[test]
fn test_negative_signal_mixed_category() {
    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
            negative_signal_weight: Some(-1.0),
            ..Default::default()
        },
        threshold: DreamingThresholdConfig {
            absolute: Some(-100.0),
            relative: Some(0.0),
        },
        capacity: DreamingCapacityConfig {
            max_rules: Some(100),
        },
        ..Default::default()
    });
    // Same-category group (no negative signal)
    let mut e1 = make_entry(EntryCategory::Error, "err1", "s1", 10);
    e1.entity_type = "subject".into();
    e1.entity_name = "same".into();
    let mut e2 = make_entry(EntryCategory::Error, "err2", "s1", 5);
    e2.entity_type = "subject".into();
    e2.entity_name = "same".into();
    // Mixed-category group (negative signal)
    let mut e3 = make_entry(EntryCategory::Error, "err3", "s1", 10);
    e3.entity_type = "subject".into();
    e3.entity_name = "mixed".into();
    let mut e4 = make_entry(EntryCategory::Decision, "dec1", "s1", 5);
    e4.entity_type = "subject".into();
    e4.entity_name = "mixed".into();
    let groups = vec![
        EntityGroup {
            entity_name: "same".into(),
            entity_type: "subject".into(),
            entries: vec![e1, e2],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
        EntityGroup {
            entity_name: "mixed".into(),
            entity_type: "subject".into(),
            entries: vec![e3, e4],
            frequency: 1,
            cross_agent_count: 1,
            score: 0.0,
        },
    ];
    let deep = pipeline.deep_stage(groups);
    let same = deep.iter().find(|g| g.entity_name == "same").unwrap();
    let mixed = deep.iter().find(|g| g.entity_name == "mixed").unwrap();
    assert!(
        same.score > mixed.score,
        "same-category {} should score higher than mixed {}",
        same.score,
        mixed.score
    );
}

/// Single entry group → negative_signal = 0.0.
#[test]
fn test_negative_signal_single_entry() {
    let pipeline = DreamingPipeline::with_config(DreamingConfig {
        scoring: DreamingScoringConfig {
            frequency_weight: Some(1.0),
            recency_weight: Some(0.0),
            explicitness_weight: Some(0.0),
            entity_type_weight_weight: Some(0.0),
            cross_agent_weight: Some(0.0),
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
    });
    let mut e1 = make_entry(EntryCategory::Error, "err1", "s1", 10);
    e1.entity_type = "subject".into();
    e1.entity_name = "solo".into();
    let groups = vec![EntityGroup {
        entity_name: "solo".into(),
        entity_type: "subject".into(),
        entries: vec![e1],
        frequency: 1,
        cross_agent_count: 1,
        score: 0.0,
    }];
    let deep = pipeline.deep_stage(groups);
    assert_eq!(deep.len(), 1);
    assert!(deep[0].score > 0.0, "single entry score: {}", deep[0].score);
}

// ── Dream Diary LLM narrative tests ─────────────────────────────

/// Mock LLM that returns prose for diary.
struct MockDiaryLlm;

#[async_trait::async_trait]
impl DreamingLlmCaller for MockDiaryLlm {
    async fn consolidate_lessons(
        &self,
        lessons: &[String],
        _entity_name: &str,
        _entity_type: &str,
        _frequency: usize,
    ) -> Result<String, DreamingLlmError> {
        Ok(lessons.join("; "))
    }

    async fn generate_diary_narrative(
        &self,
        promoted_groups: &[PromotedGroupInfo],
    ) -> Result<String, DreamingLlmError> {
        let names: Vec<&str> = promoted_groups
            .iter()
            .map(|g| g.entity_name.as_str())
            .collect();
        Ok(format!("Narrative about {}.", names.join(" and ")))
    }
}

/// Failing LLM for diary degradation test.
struct FailingDiaryLlm;

#[async_trait::async_trait]
impl DreamingLlmCaller for FailingDiaryLlm {
    async fn consolidate_lessons(
        &self,
        _lessons: &[String],
        _entity_name: &str,
        _entity_type: &str,
        _frequency: usize,
    ) -> Result<String, DreamingLlmError> {
        Err(DreamingLlmError::Llm("fail".into()))
    }

    async fn generate_diary_narrative(
        &self,
        _promoted_groups: &[PromotedGroupInfo],
    ) -> Result<String, DreamingLlmError> {
        Err(DreamingLlmError::Llm("diary fail".into()))
    }
}

/// Dream Diary with LLM → prose format.
#[tokio::test]
async fn test_dream_diary_llm_narrative_success() {
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
    let llm = MockDiaryLlm;
    let promoted = vec![PromotedGroupInfo {
        entity_name: "deploy".into(),
        entity_type: "subject".into(),
        lessons: vec!["always verify".into()],
    }];
    pipeline
        .write_dream_diary(&promoted, Some(&llm))
        .await
        .unwrap();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let content = std::fs::read_to_string(tmp.path().join(format!("{}.md", date))).unwrap();
    assert!(
        content.contains("Narrative about deploy"),
        "diary should contain LLM prose, got: {}",
        content
    );
}

/// Dream Diary with failing LLM → fallback to structured summary.
#[tokio::test]
async fn test_dream_diary_llm_failure_fallback() {
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
    let llm = FailingDiaryLlm;
    let promoted = vec![PromotedGroupInfo {
        entity_name: "deploy".into(),
        entity_type: "subject".into(),
        lessons: vec!["verify before deploy".into()],
    }];
    pipeline
        .write_dream_diary(&promoted, Some(&llm))
        .await
        .unwrap();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let content = std::fs::read_to_string(tmp.path().join(format!("{}.md", date))).unwrap();
    // Fallback should produce bullet list format.
    assert!(
        content.contains("- **deploy** (subject): verify before deploy"),
        "diary should fallback to structured summary, got: {}",
        content
    );
}

/// Dream Diary with no LLM → fallback to structured summary.
#[tokio::test]
async fn test_dream_diary_no_llm_fallback() {
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
    let promoted = vec![PromotedGroupInfo {
        entity_name: "vim".into(),
        entity_type: "subject".into(),
        lessons: vec!["use vim".into()],
    }];
    pipeline.write_dream_diary(&promoted, None).await.unwrap();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let content = std::fs::read_to_string(tmp.path().join(format!("{}.md", date))).unwrap();
    assert!(content.contains("- **vim** (subject): use vim"));
}

/// Empty promoted list → no diary file created.
#[tokio::test]
async fn test_dream_diary_empty_promoted_no_write() {
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
    pipeline.write_dream_diary(&[], None).await.unwrap();
    assert!(
        tmp.path().read_dir().unwrap().next().is_none(),
        "no diary file should be created for empty promoted list"
    );
}

// ── Integration: run_once diary contains only promoted groups ────

/// Integration test: run_once end-to-end — diary only contains promoted groups.
#[tokio::test]
async fn test_run_once_diary_only_promoted_groups() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("int.db");
    let diary_dir = tmp.path().join("diary");
    let memory_md = tmp.path().join("MEMORY.md");

    // Create DB with events + entities.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT,
             content TEXT NOT NULL, category TEXT NOT NULL, lesson TEXT,
             source_session_id TEXT NOT NULL, timestamp INTEGER NOT NULL,
             updated_at INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE entities (id INTEGER PRIMARY KEY AUTOINCREMENT,
             agent_id TEXT NOT NULL, type TEXT NOT NULL, name TEXT NOT NULL,
             normalized_name TEXT NOT NULL,
             UNIQUE(agent_id, type, normalized_name));
             CREATE TABLE event_entities (id INTEGER PRIMARY KEY AUTOINCREMENT,
             event_id INTEGER NOT NULL, entity_id INTEGER NOT NULL);
             INSERT INTO events (content, category, lesson, source_session_id,
             timestamp, updated_at)
             VALUES ('err1', 'error', 'lesson1', 'sess-1', 1700000000, 1700000000);
             INSERT INTO events (content, category, lesson, source_session_id,
             timestamp, updated_at)
             VALUES ('err2', 'error', 'lesson2', 'sess-1', 1700000060, 1700000060);
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('a1', 'subject', 'EntityA', 'entitya');
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('a1', 'subject', 'EntityB', 'entityb');
             INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1);
             INSERT INTO event_entities (event_id, entity_id) VALUES (2, 2);",
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
            enabled: Some(true),
            path: Some(diary_dir.to_str().unwrap().to_string()),
        },
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

    let llm: std::sync::Arc<dyn DreamingLlmCaller> = std::sync::Arc::new(MockDiaryLlm);

    let pipeline = DreamingPipeline::with_config(config)
        .with_db_path(&db_path)
        .with_memory_md_path(memory_md.to_str().unwrap())
        .with_llm(llm);

    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once failed: {result:?}");

    // Diary should exist and contain LLM narrative (only for promoted groups).
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let diary_file = diary_dir.join(format!("{}.md", date));
    assert!(diary_file.exists(), "diary file should be created");
    let content = std::fs::read_to_string(&diary_file).unwrap();
    // LLM narrative should reference entity names from promoted groups.
    assert!(
        content.contains("Narrative about"),
        "diary should contain LLM narrative, got: {}",
        content
    );
    // Session should be marked Completed.
    let cps = storage.checkpoints.lock().unwrap();
    let cp = cps.iter().find(|c| c.session_id == "sess-1").unwrap();
    assert_eq!(cp.dreaming_status, DreamingStatus::Completed);
}
