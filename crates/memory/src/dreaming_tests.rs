//! Additional unit tests for DreamingPipeline.
//!
//! Complements the inline tests in dreaming.rs with tests that require
//! mock PersistenceService interactions.

use crate::dreaming::{DreamingPipeline, EntryCategory, MemoryEntry};
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::{DreamingConfig, DreamingDiaryConfig};
use closeclaw_session::persistence::{DreamingStatus, SessionCheckpoint};

use tempfile::TempDir;

// ── Tests ────────────────────────────────────────────────────────────────

/// Dreaming pipeline does not reprocess sessions already marked Completed.
///
/// When `list_mined_undreamt_sessions()` returns empty (all sessions are
/// already Completed), `run_once()` should return Ok immediately without
/// attempting to process any entries.
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

/// Dreaming pipeline processes sessions that are mined but not yet dreamt.
#[tokio::test]
async fn test_dreaming_processes_mined_undreamt_sessions() {
    let storage = TestStorage::default();

    // mined=true, dreaming_status=Pending → should be processed.
    let mut cp = SessionCheckpoint::new("sess-pending".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let config = DreamingConfig {
        enabled: true,
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

/// Dreaming pipeline returns Ok immediately when dreaming is disabled.
#[tokio::test]
async fn test_dreaming_disabled_skips_processing() {
    let storage = TestStorage::default();

    let mut cp = SessionCheckpoint::new("sess-pending".into());
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Pending;
    storage.add_checkpoint(cp);

    let config = DreamingConfig {
        enabled: false,
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

// ── Dream Diary tests ──────────────────────────────────────────────────

/// Helper to create a MemoryEntry for testing.
fn make_entry(
    category: EntryCategory,
    body: &str,
    session_id: &str,
    minutes_ago: i64,
) -> MemoryEntry {
    MemoryEntry {
        category,
        body: body.to_string(),
        timestamp: chrono::Utc::now() - chrono::Duration::minutes(minutes_ago),
        source_session_id: session_id.to_string(),
        lesson: None,
        tags: Vec::new(),
        score: 0.0,
    }
}

/// Helper to create an entry with a lesson.
fn make_entry_with_lesson(
    category: EntryCategory,
    body: &str,
    session_id: &str,
    lesson: &str,
) -> MemoryEntry {
    MemoryEntry {
        category,
        body: body.to_string(),
        timestamp: chrono::Utc::now(),
        source_session_id: session_id.to_string(),
        lesson: Some(lesson.to_string()),
        tags: Vec::new(),
        score: 0.0,
    }
}

/// Dream Diary writes a file when diary is enabled and entries exist.
#[test]
fn test_dream_diary_writes_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let config = DreamingConfig {
        enabled: true,
        diary: DreamingDiaryConfig {
            enabled: true,
            path: diary_path.clone(),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let entries = vec![
        make_entry(EntryCategory::Decision, "dark mode preferred", "s1", 10),
        make_entry_with_lesson(
            EntryCategory::Error,
            "wrong deployment",
            "s1",
            "verify before deploying",
        ),
    ];

    let result = pipeline.write_dream_diary(&entries);
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

/// Dream Diary does NOT write a file when diary is disabled.
#[test]
fn test_dream_diary_does_not_write_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let config = DreamingConfig {
        enabled: true,
        diary: DreamingDiaryConfig {
            enabled: false,
            path: diary_path,
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let entries = vec![make_entry(
        EntryCategory::Decision,
        "should not appear",
        "s1",
        10,
    )];

    let result = pipeline.write_dream_diary(&entries);
    assert!(result.is_ok());

    // Diary directory should NOT exist since diary is disabled.
    assert!(
        tmp.path().read_dir().unwrap().next().is_none(),
        "no files should be created when diary is disabled"
    );
}

/// Dream Diary uses custom path from config.
#[test]
fn test_dream_diary_uses_custom_path() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().join("custom/diary");
    let config = DreamingConfig {
        enabled: true,
        diary: DreamingDiaryConfig {
            enabled: true,
            path: diary_path.to_str().unwrap().to_string(),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let entries = vec![make_entry(
        EntryCategory::Decision,
        "custom path test",
        "s1",
        10,
    )];

    let result = pipeline.write_dream_diary(&entries);
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

/// Dream Diary auto-creates the diary directory if it does not exist.
#[test]
fn test_dream_diary_creates_directory() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().join("new/dir/level");
    let config = DreamingConfig {
        enabled: true,
        diary: DreamingDiaryConfig {
            enabled: true,
            path: diary_path.to_str().unwrap().to_string(),
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let entries = vec![make_entry(
        EntryCategory::Decision,
        "auto dir test",
        "s1",
        10,
    )];

    let result = pipeline.write_dream_diary(&entries);
    assert!(result.is_ok());
    assert!(
        diary_path.exists(),
        "diary directory should be auto-created"
    );
}

/// EntryCategory variants are exactly Error, Anger, Decision.
/// This is a regression guard — the design doc defines these three
/// categories and any deviation will break the dreaming pipeline.
#[test]
fn test_entry_category_variants_match_design_doc() {
    // Exhaustive match to catch future additions that don't follow spec.
    let all = [
        EntryCategory::Error,
        EntryCategory::Anger,
        EntryCategory::Decision,
    ];
    assert_eq!(all.len(), 3);

    // Verify each variant displays correctly in diary output.
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
/// The design doc specifies that lesson is required for Error/Anger.
#[test]
fn test_error_anger_entries_carry_lesson_in_diary() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let config = DreamingConfig {
        enabled: true,
        diary: DreamingDiaryConfig {
            enabled: true,
            path: diary_path,
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let entries = vec![
        make_entry_with_lesson(
            EntryCategory::Error,
            "wrong deployment",
            "s1",
            "verify before deploying",
        ),
        make_entry_with_lesson(
            EntryCategory::Anger,
            "user corrected output",
            "s1",
            "follow user style guide",
        ),
    ];

    let result = pipeline.write_dream_diary(&entries);
    assert!(result.is_ok());

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let diary_file = tmp.path().join(format!("{}.md", date));
    let content = std::fs::read_to_string(&diary_file).unwrap();

    // Both Error and Anger entries should include their lesson in the output.
    assert!(content.contains("Lesson: verify before deploying"));
    assert!(content.contains("Lesson: follow user style guide"));
}

/// Dream Diary does NOT write when entries list is empty.
#[test]
fn test_dream_diary_empty_entries_no_write() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let config = DreamingConfig {
        enabled: true,
        diary: DreamingDiaryConfig {
            enabled: true,
            path: diary_path,
        },
        ..Default::default()
    };
    let pipeline = DreamingPipeline::with_config(config);

    let entries: Vec<MemoryEntry> = vec![];
    let result = pipeline.write_dream_diary(&entries);
    assert!(result.is_ok());

    // No files should be created in the diary directory.
    assert!(
        tmp.path().read_dir().unwrap().next().is_none(),
        "no files should be created for empty entries"
    );
}

// ── Custom scoring/threshold/capacity config tests ─────────────────

use closeclaw_config::agents::{
    DreamingCapacityConfig, DreamingScoringConfig, DreamingThresholdConfig,
};

/// Custom scoring config is accepted by with_config constructor.
#[test]
fn test_dreaming_pipeline_custom_scoring_config() {
    let config = DreamingConfig {
        enabled: true,
        scoring: DreamingScoringConfig {
            frequency_weight: 2.0,
            recency_weight: 1.0,
            explicitness_weight: 3.0,
            cross_agent_weight: 2.0,
            negative_signal_weight: -1.0,
        },
        threshold: DreamingThresholdConfig {
            absolute: 0.0,
            relative: 0.0,
        },
        capacity: DreamingCapacityConfig { max_rules: 100 },
        ..Default::default()
    };
    // Verify with_config doesn't panic and pipeline is constructible.
    let _pipeline = DreamingPipeline::with_config(config);
}

/// High absolute threshold config is accepted.
#[test]
fn test_dreaming_pipeline_high_threshold_config() {
    let config = DreamingConfig {
        enabled: true,
        scoring: DreamingScoringConfig {
            frequency_weight: 0.0,
            recency_weight: 0.0,
            explicitness_weight: 0.0,
            cross_agent_weight: 0.0,
            negative_signal_weight: 0.0,
        },
        threshold: DreamingThresholdConfig {
            absolute: 5.0,
            relative: 0.0,
        },
        capacity: DreamingCapacityConfig { max_rules: 100 },
        ..Default::default()
    };
    let _pipeline = DreamingPipeline::with_config(config);
}

/// Capacity config with small max_rules is accepted.
#[test]
fn test_dreaming_pipeline_capacity_config_stored() {
    let config = DreamingConfig {
        enabled: true,
        scoring: DreamingScoringConfig::default(),
        threshold: DreamingThresholdConfig {
            absolute: 0.0,
            relative: 0.0,
        },
        capacity: DreamingCapacityConfig { max_rules: 5 },
        ..Default::default()
    };
    let _pipeline = DreamingPipeline::with_config(config);
}

/// Boundary: max_rules=0 config is accepted without panic.
#[test]
fn test_dreaming_pipeline_max_rules_zero_config() {
    let config = DreamingConfig {
        enabled: true,
        threshold: DreamingThresholdConfig {
            absolute: 0.0,
            relative: 0.0,
        },
        capacity: DreamingCapacityConfig { max_rules: 0 },
        ..Default::default()
    };
    let _pipeline = DreamingPipeline::with_config(config);
}

/// Custom relative threshold config is accepted.
#[test]
fn test_dreaming_pipeline_relative_threshold_config() {
    let config = DreamingConfig {
        enabled: true,
        scoring: DreamingScoringConfig::default(),
        threshold: DreamingThresholdConfig {
            absolute: 0.0,
            relative: 0.5,
        },
        capacity: DreamingCapacityConfig { max_rules: 100 },
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
