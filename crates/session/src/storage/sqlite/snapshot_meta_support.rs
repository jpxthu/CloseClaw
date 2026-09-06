//! Snapshot metadata persistence helpers for SQLite.
//!
//! These helpers are used by SqliteStorage for snapshot metadata
//! save/load operations. Kept separate to keep sqlite.rs under 1000 lines.

use crate::persistence::PersistenceError;
use crate::run_health::{SnapshotMeta, SnapshotStatus};
use rusqlite::{params, Connection};

/// Save snapshot metadata for a session. Replaces all existing metas
/// atomically (delete + insert).
pub fn save_snapshot_metas_inner(
    conn: &Connection,
    session_id: &str,
    metas: &[SnapshotMeta],
) -> Result<(), PersistenceError> {
    conn.execute(
        "DELETE FROM snapshot_metas WHERE session_id = ?1",
        params![session_id],
    )
    .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    for meta in metas {
        let status_str = match meta.status {
            SnapshotStatus::Pending => "pending",
            SnapshotStatus::Complete => "complete",
        };
        conn.execute(
            "INSERT INTO snapshot_metas (id, session_id, reason, created_at, status) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                meta.id,
                session_id,
                meta.reason,
                meta.created_at.to_rfc3339(),
                status_str,
            ],
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    }
    Ok(())
}

/// Load snapshot metadata for a session, ordered by creation time.
pub fn load_snapshot_metas_inner(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<SnapshotMeta>, PersistenceError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, reason, created_at, status \
             FROM snapshot_metas WHERE session_id = ?1 \
             ORDER BY created_at",
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    let metas: Vec<SnapshotMeta> = stmt
        .query_map(params![session_id], |row| {
            let id: String = row.get(0)?;
            let reason: String = row.get(1)?;
            let created_at_str: String = row.get(2)?;
            let status_str: String = row.get(3)?;
            Ok((id, reason, created_at_str, status_str))
        })
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .map(|(id, reason, created_at_str, status_str)| {
            let status = match status_str.as_str() {
                "complete" => SnapshotStatus::Complete,
                _ => SnapshotStatus::Pending,
            };
            let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            SnapshotMeta {
                id,
                reason,
                created_at,
                session_id: session_id.to_string(),
                status,
            }
        })
        .collect();

    Ok(metas)
}
