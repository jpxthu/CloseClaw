//! Tests for SQLite ↔ file-system consistency check helpers.

use super::consistency_check::{
    check_filesystem_to_sqlite_filtered, check_sqlite_to_filesystem_filtered,
};
use crate::persistence::ConsistencyCheckResult;
use rusqlite::Connection;
use std::fs;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal schema matching the production `sessions` table.
fn create_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            role TEXT NOT NULL,
            channel TEXT NOT NULL,
            chat_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            title TEXT,
            last_message_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            archived_at INTEGER,
            message_count INTEGER DEFAULT 0,
            metadata TEXT
        );",
    )
    .unwrap();
}

/// Insert a session record directly into SQLite.
fn insert_session(
    conn: &Connection,
    id: &str,
    status: &str,
    last_message_at: i64,
    archived_at: Option<i64>,
) {
    conn.execute(
        "INSERT INTO sessions \
         (id, agent_id, role, channel, chat_id, status, title, \
          last_message_at, created_at, archived_at, metadata) \
         VALUES (?1, 'agent', 'main', 'feishu', 'chat', ?2, 'title', ?3, ?4, ?5, '{}')",
        rusqlite::params![id, status, last_message_at, last_message_at, archived_at],
    )
    .unwrap();
}

/// Create an empty `.jsonl` file (mimics a transcript file).
fn create_jsonl(dir: &std::path::Path, session_id: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(format!("{session_id}.jsonl")), "").unwrap();
}

// ---------------------------------------------------------------------------
// SQLite → Filesystem tests
// ---------------------------------------------------------------------------

/// Normal path: archived session has both SQLite record and transcript file.
/// Nothing should be deleted.
#[test]
fn test_sqlite_to_fs_archived_normal_path() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    insert_session(&conn, "s1", "archived", 1000, Some(1000));
    create_jsonl(&data_dir.join("archived_sessions"), "s1");

    let mut result = ConsistencyCheckResult::default();
    check_sqlite_to_filesystem_filtered(&conn, data_dir, &mut result, 0).unwrap();

    assert_eq!(
        result.deleted_orphaned_records, 0,
        "no records should be deleted"
    );
    // File should still exist
    assert!(data_dir.join("archived_sessions/s1.jsonl").exists());
}

/// Corrupted record: SQLite has archived record but no transcript file.
/// The record should be deleted.
#[test]
fn test_sqlite_to_fs_archived_corrupted_record() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    insert_session(&conn, "s2", "archived", 1000, Some(1000));
    // Do NOT create the file

    let mut result = ConsistencyCheckResult::default();
    check_sqlite_to_filesystem_filtered(&conn, data_dir, &mut result, 0).unwrap();

    assert_eq!(
        result.deleted_orphaned_records, 1,
        "one record should be deleted"
    );
    // Verify record was removed from SQLite
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            rusqlite::params!["s2"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "archived record without file should be removed");
}

/// Incremental scan: archived_at <= since → record is skipped.
#[test]
fn test_sqlite_to_fs_archived_incremental_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    // archived_at = 1000, since = 2000 → should be skipped
    insert_session(&conn, "s3", "archived", 1000, Some(1000));
    // No file created, but since > archived_at so it's skipped

    let mut result = ConsistencyCheckResult::default();
    check_sqlite_to_filesystem_filtered(&conn, data_dir, &mut result, 2000).unwrap();

    assert_eq!(
        result.deleted_orphaned_records, 0,
        "record with archived_at <= since should be skipped"
    );
    // Record should still exist
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            rusqlite::params!["s3"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "record should remain when archived_at <= since");
}

/// Incremental scan: archived_at > since → record IS checked.
#[test]
fn test_sqlite_to_fs_archived_incremental_checked() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    // archived_at = 5000, since = 2000 → should be checked
    insert_session(&conn, "s4", "archived", 5000, Some(5000));
    // No file → should be deleted

    let mut result = ConsistencyCheckResult::default();
    check_sqlite_to_filesystem_filtered(&conn, data_dir, &mut result, 2000).unwrap();

    assert_eq!(
        result.deleted_orphaned_records, 1,
        "record with archived_at > since should be checked and deleted"
    );
}

/// Both active and archived orphans exist simultaneously → both cleaned up.
#[test]
fn test_sqlite_to_fs_both_active_and_archived_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    // Active orphan: no sessions/a1.jsonl
    insert_session(&conn, "a1", "active", 5000, None);
    // Archived orphan: no archived_sessions/a2.jsonl
    insert_session(&conn, "a2", "archived", 5000, Some(5000));

    let mut result = ConsistencyCheckResult::default();
    check_sqlite_to_filesystem_filtered(&conn, data_dir, &mut result, 0).unwrap();

    assert_eq!(
        result.deleted_orphaned_records, 2,
        "both active and archived orphans should be deleted"
    );
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "all orphan records should be removed");
}

// ---------------------------------------------------------------------------
// Filesystem → SQLite tests
// ---------------------------------------------------------------------------

/// Normal path: orphan file exists in archived_sessions/ with matching SQLite
/// record → file should NOT be deleted.
#[test]
fn test_fs_to_sqlite_archived_normal_path() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    insert_session(&conn, "f1", "archived", 1000, Some(1000));
    create_jsonl(&data_dir.join("archived_sessions"), "f1");

    let mut result = ConsistencyCheckResult::default();
    check_filesystem_to_sqlite_filtered(&conn, data_dir, &mut result, 0).unwrap();

    assert_eq!(
        result.deleted_orphaned_files, 0,
        "no files should be deleted"
    );
    assert!(data_dir.join("archived_sessions/f1.jsonl").exists());
}

/// Orphan file: archived_sessions/ has .jsonl but no SQLite record → file
/// should be deleted.
#[test]
fn test_fs_to_sqlite_archived_orphan_file() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    // Do NOT insert a SQLite record
    create_jsonl(&data_dir.join("archived_sessions"), "f2");

    let mut result = ConsistencyCheckResult::default();
    check_filesystem_to_sqlite_filtered(&conn, data_dir, &mut result, 0).unwrap();

    assert_eq!(
        result.deleted_orphaned_files, 1,
        "one orphan file should be deleted"
    );
    assert!(
        !data_dir.join("archived_sessions/f2.jsonl").exists(),
        "orphan file should be removed"
    );
}

/// Both active and archived orphan files → both deleted.
#[test]
fn test_fs_to_sqlite_both_active_and_archived_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    create_jsonl(&data_dir.join("sessions"), "b1");
    create_jsonl(&data_dir.join("archived_sessions"), "b2");

    let mut result = ConsistencyCheckResult::default();
    check_filesystem_to_sqlite_filtered(&conn, data_dir, &mut result, 0).unwrap();

    assert_eq!(
        result.deleted_orphaned_files, 2,
        "both active and archived orphan files should be deleted"
    );
    assert!(!data_dir.join("sessions/b1.jsonl").exists());
    assert!(!data_dir.join("archived_sessions/b2.jsonl").exists());
}

/// Incremental scan: file mtime <= since → file is skipped.
#[test]
fn test_fs_to_sqlite_archived_incremental_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    // Create an orphan file with an old mtime (before since)
    create_jsonl(&data_dir.join("archived_sessions"), "f3");
    // Set mtime to 500 (older than since=2000)
    let file_path = data_dir.join("archived_sessions/f3.jsonl");
    let ft = filetime::FileTime::from_unix_time(500, 0);
    filetime::set_file_mtime(&file_path, ft).unwrap();

    let mut result = ConsistencyCheckResult::default();
    check_filesystem_to_sqlite_filtered(&conn, data_dir, &mut result, 2000).unwrap();

    assert_eq!(
        result.deleted_orphaned_files, 0,
        "file with mtime <= since should be skipped"
    );
    assert!(file_path.exists(), "file should remain when mtime <= since");
}

/// Non-jsonl files in archived_sessions/ are ignored.
#[test]
fn test_fs_to_sqlite_archived_non_jsonl_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let data_dir = tmp.path();

    let dir = data_dir.join("archived_sessions");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("s1.txt"), "").unwrap();

    let mut result = ConsistencyCheckResult::default();
    check_filesystem_to_sqlite_filtered(&conn, data_dir, &mut result, 0).unwrap();

    assert_eq!(
        result.deleted_orphaned_files, 0,
        "non-jsonl files should be ignored"
    );
    assert!(dir.join("s1.txt").exists());
}
