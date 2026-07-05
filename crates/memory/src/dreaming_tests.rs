//! Additional unit tests for DreamingPipeline.
//!
//! Complements the inline tests in dreaming.rs with tests that require
//! mock PersistenceService interactions.

use crate::dreaming::{DreamingPipeline, EntryCategory, MemoryEntry};
use crate::test_helpers::TestStorage;
use closeclaw_config::agents::DreamingDiaryConfig;
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

    let pipeline = DreamingPipeline::new();
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
    let diary_config = DreamingDiaryConfig {
        enabled: true,
        path: diary_path.clone(),
    };
    let pipeline = DreamingPipeline::with_diary_config(diary_config);

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
/// Testing through run_once: with no undreamt sessions, run_once
/// returns Ok immediately without touching the diary directory.
#[tokio::test]
async fn test_dream_diary_does_not_write_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let diary_config = DreamingDiaryConfig {
        enabled: false,
        path: diary_path,
    };
    let pipeline = DreamingPipeline::with_diary_config(diary_config);
    let storage = TestStorage::default();

    let result = pipeline.run_once(&storage).await;
    assert!(result.is_ok(), "run_once should succeed: {result:?}");

    // Diary directory should NOT exist since no sessions were processed.
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
    let diary_config = DreamingDiaryConfig {
        enabled: true,
        path: diary_path.to_str().unwrap().to_string(),
    };
    let pipeline = DreamingPipeline::with_diary_config(diary_config);

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
    let diary_config = DreamingDiaryConfig {
        enabled: true,
        path: diary_path.to_str().unwrap().to_string(),
    };
    let pipeline = DreamingPipeline::with_diary_config(diary_config);

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

/// Dream Diary does NOT write when entries list is empty.
#[test]
fn test_dream_diary_empty_entries_no_write() {
    let tmp = TempDir::new().unwrap();
    let diary_path = tmp.path().to_str().unwrap().to_string();
    let diary_config = DreamingDiaryConfig {
        enabled: true,
        path: diary_path,
    };
    let pipeline = DreamingPipeline::with_diary_config(diary_config);

    let entries: Vec<MemoryEntry> = vec![];
    let result = pipeline.write_dream_diary(&entries);
    assert!(result.is_ok());

    // No files should be created in the diary directory.
    assert!(
        tmp.path().read_dir().unwrap().next().is_none(),
        "no files should be created for empty entries"
    );
}
