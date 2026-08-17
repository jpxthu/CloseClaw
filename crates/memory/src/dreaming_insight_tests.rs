//! Insight category flow tests for dreaming pipeline.
//!
//! Verifies that load_entries_from_sqlite correctly handles the "insight"
//! category, which was previously silently skipped.

use crate::dreaming::{DreamingPipeline, EntryCategory};
use rusqlite::params;

/// load_entries_from_sqlite correctly parses category="insight" into
/// EntryCategory::Insight (not skipped by the `_ => continue` branch).
#[test]
fn test_load_entries_insight_category_flow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("insight.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, mined INTEGER NOT NULL DEFAULT 0);
             INSERT INTO sessions (id, mined) VALUES ('sess-insight', 1);
             CREATE TABLE events (
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
             VALUES ('insight body', 'insight', NULL, 'sess-insight', 1700000000, 1700000000);
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('a1', 'subject', 'Pattern', 'pattern');
             INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1);",
        )
        .unwrap();
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let entries = pipeline
        .load_entries_from_sqlite(&conn, "sess-insight")
        .unwrap();

    assert_eq!(entries.len(), 1, "insight event should be returned");
    assert_eq!(entries[0].category, EntryCategory::Insight);
    assert_eq!(entries[0].body, "insight body");
    assert_eq!(entries[0].event_id, 1);
}

/// load_entries_from_sqlite parses all four known categories correctly.
#[test]
fn test_load_entries_all_known_categories() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("all_cat.db");
    let categories = ["error", "anger", "decision", "insight"];
    let expected = [
        EntryCategory::Error,
        EntryCategory::Anger,
        EntryCategory::Decision,
        EntryCategory::Insight,
    ];
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, mined INTEGER NOT NULL DEFAULT 0);
             INSERT INTO sessions (id, mined) VALUES ('sess-all', 1);
             CREATE TABLE events (
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
                event_id INTEGER NOT NULL, entity_id INTEGER NOT NULL);",
        )
        .unwrap();
        for (i, cat) in categories.iter().enumerate() {
            conn.execute(
                "INSERT INTO events (content, category, lesson, source_session_id,
                  timestamp, updated_at)
                 VALUES (?1, ?2, NULL, 'sess-all', 1700000000, 1700000000)",
                params![format!("body {}", i), cat],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO entities (agent_id, type, name, normalized_name)
                 VALUES ('a1', 'subject', ?1, ?1)",
                params![format!("entity_{}", i)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
                params![i + 1, i + 1],
            )
            .unwrap();
        }
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let entries = pipeline
        .load_entries_from_sqlite(&conn, "sess-all")
        .unwrap();

    assert_eq!(entries.len(), 4, "all four categories should be returned");
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(entry.category, expected[i]);
    }
}

/// Unknown category is skipped (the `_ => continue` branch).
#[test]
fn test_load_entries_unknown_category_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("unknown_cat.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, mined INTEGER NOT NULL DEFAULT 0);
             INSERT INTO sessions (id, mined) VALUES ('sess-unk', 1);
             CREATE TABLE events (
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
             VALUES ('body', 'unknown_category', NULL, 'sess-unk', 1700000000, 1700000000);
             INSERT INTO entities (agent_id, type, name, normalized_name)
             VALUES ('a1', 'subject', 'Ent', 'ent');
             INSERT INTO event_entities (event_id, entity_id) VALUES (1, 1);",
        )
        .unwrap();
    }
    let pipeline = DreamingPipeline::new().with_db_path(&db_path);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let entries = pipeline
        .load_entries_from_sqlite(&conn, "sess-unk")
        .unwrap();
    assert!(entries.is_empty(), "unknown category should be skipped");
}
