//! SQLite ↔ file-system consistency check helpers.
//!
//! Extracted from `sqlite.rs` to keep the main module under the
//! project's 1000-line file limit.

use crate::persistence::{ConsistencyCheckResult, PersistenceError};
use rusqlite::Connection;

/// SQLite → File system: check active and archived records.
///
/// For active records (`last_message_at > since`), verify
/// `sessions/{id}.jsonl` exists.  For archived records
/// (`archived_at > since`), verify `archived_sessions/{id}.jsonl`
/// exists.  Missing transcript files trigger SQLite record deletion.
pub(super) fn check_sqlite_to_filesystem_filtered(
    conn: &Connection,
    data_dir: &std::path::Path,
    result: &mut ConsistencyCheckResult,
    since: i64,
) -> Result<(), PersistenceError> {
    let active_ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM sessions \
                 WHERE status = 'active' AND last_message_at > ?1",
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![since], |row| row.get(0))
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    for id in &active_ids {
        let transcript_path = data_dir.join("sessions").join(format!("{id}.jsonl"));
        if !transcript_path.exists() {
            conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            result.deleted_orphaned_records += 1;
        }
    }

    let archived_ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM sessions \
                 WHERE status = 'archived' AND archived_at > ?1",
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![since], |row| row.get(0))
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    for id in &archived_ids {
        let transcript_path = data_dir
            .join("archived_sessions")
            .join(format!("{id}.jsonl"));
        if !transcript_path.exists() {
            conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            result.deleted_orphaned_records += 1;
        }
    }

    Ok(())
}

/// Scan a single directory for `.jsonl` files orphaned in SQLite.
fn scan_dir_for_orphans(
    conn: &Connection,
    dir: &std::path::Path,
    result: &mut ConsistencyCheckResult,
    since: i64,
) -> Result<(), PersistenceError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|e| PersistenceError::Sqlite(e.to_string()))? {
        let entry = entry.map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if mtime <= since {
            continue;
        }
        let file_stem = path.file_stem().and_then(|e| e.to_str()).unwrap_or("");
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sessions WHERE id = ?1",
                rusqlite::params![file_stem],
                |row| row.get(0),
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        if !exists {
            std::fs::remove_file(&path).map_err(PersistenceError::Io)?;
            result.deleted_orphaned_files += 1;
        }
    }
    Ok(())
}

/// File system → SQLite: check `.jsonl` files whose mtime > since.
///
/// Scans both `sessions/` (active) and `archived_sessions/` (archived)
/// directories.  Orphaned files (no matching SQLite record) are deleted.
pub(super) fn check_filesystem_to_sqlite_filtered(
    conn: &Connection,
    data_dir: &std::path::Path,
    result: &mut ConsistencyCheckResult,
    since: i64,
) -> Result<(), PersistenceError> {
    scan_dir_for_orphans(conn, &data_dir.join("sessions"), result, since)?;
    scan_dir_for_orphans(conn, &data_dir.join("archived_sessions"), result, since)?;
    Ok(())
}
