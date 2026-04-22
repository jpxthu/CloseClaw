//! SQLite archive operations helpers
//!
//! These helpers are used by SqliteStorage for archive/restore/purge/list
//! operations. Kept separate to keep sqlite.rs under 500 lines.

use crate::session::persistence::{PersistenceError, SessionCheckpoint};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;

/// Begin an immediate transaction, or return the error.
pub fn begin_immediate(conn: &Connection) -> Result<(), PersistenceError> {
    conn.execute("BEGIN IMMEDIATE", [])
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    Ok(())
}

/// Commit the transaction, rolling back on error.
pub fn commit(conn: &Connection) -> Result<(), PersistenceError> {
    conn.execute("COMMIT", []).map_err(|e| {
        let _ = conn.execute("ROLLBACK", []);
        PersistenceError::Sqlite(e.to_string())
    })?;
    Ok(())
}

/// Rollback the transaction.
pub fn rollback(conn: &Connection) {
    let _ = conn.execute("ROLLBACK", []);
}

/// Rename a transcript file, returning an Io error on failure.
pub fn rename_transcript(from: &Path, to: &Path) -> Result<(), PersistenceError> {
    std::fs::rename(from, to).map_err(PersistenceError::Io)
}

/// Delete a transcript file, ignoring "not found".
pub fn delete_transcript(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Archive a checkpoint: move its active transcript to archived_sessions/
/// and mark the DB record as archived.
pub fn do_archive(data_dir: &Path, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError> {
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    begin_immediate(&conn)?;

    // Idempotent: if not active, commit no-op
    let active: bool = match conn.query_row(
        "SELECT 1 FROM sessions WHERE id = ?1 AND status = 'active'",
        [&checkpoint.session_id],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(_) => true,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            // Session doesn't exist or is not active — commit no-op and return
            return commit(&conn).map(|_| ());
        }
        Err(e) => return Err(PersistenceError::Sqlite(e.to_string())),
    };
    if !active {
        return commit(&conn).map(|_| ());
    }

    let src = data_dir
        .join("sessions")
        .join(format!("{}.jsonl", checkpoint.session_id));
    let dst = data_dir
        .join("archived_sessions")
        .join(format!("{}.jsonl", checkpoint.session_id));

    rename_transcript(&src, &dst)?;

    let now = Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE sessions SET status = 'archived', archived_at = ?1 WHERE id = ?2",
        rusqlite::params![now, checkpoint.session_id],
    )
    .map_err(|e| {
        rollback(&conn);
        PersistenceError::Sqlite(e.to_string())
    })?;

    commit(&conn)
}

/// Restore an archived checkpoint: move transcript back to sessions/ and
/// mark the DB record as active.
pub fn do_restore(
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<SessionCheckpoint>, PersistenceError> {
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    begin_immediate(&conn)?;

    // Verify archived status
    let archived: bool = match conn.query_row(
        "SELECT 1 FROM sessions WHERE id = ?1 AND status = 'archived'",
        [session_id],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(_) => true,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            rollback(&conn);
            return Err(PersistenceError::NotFound(session_id.to_string()));
        }
        Err(e) => return Err(PersistenceError::Sqlite(e.to_string())),
    };

    if !archived {
        rollback(&conn);
        return Err(PersistenceError::NotFound(session_id.to_string()));
    }

    let src = data_dir
        .join("archived_sessions")
        .join(format!("{session_id}.jsonl"));
    let dst = data_dir
        .join("sessions")
        .join(format!("{session_id}.jsonl"));

    rename_transcript(&src, &dst)?;

    conn.execute(
        "UPDATE sessions SET status = 'active', archived_at = NULL WHERE id = ?1",
        [session_id],
    )
    .map_err(|e| {
        rollback(&conn);
        PersistenceError::Sqlite(e.to_string())
    })?;

    commit(&conn)?;

    // Reload from DB after restore (uses the same helper as load_checkpoint)
    load_checkpoint_inner(&conn, data_dir, session_id)
}

/// Load a SessionCheckpoint from an open DB connection.
pub fn load_checkpoint_inner(
    conn: &Connection,
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<SessionCheckpoint>, PersistenceError> {
    let mut stmt = conn
        .prepare(
            "SELECT agent_id, role, channel, chat_id, status, title,
             last_message_at, created_at, archived_at, message_count, metadata
             FROM sessions WHERE id = ?1",
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    let row = match stmt.query_row(params![session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, Option<String>>(10)?,
        ))
    }) {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(PersistenceError::Sqlite(e.to_string())),
    };

    let (
        _agent_id,
        _role,
        channel,
        chat_id,
        status_db,
        _title,
        last_msg_ts,
        created_ts,
        _archived_ts,
        msg_count,
        metadata,
    ) = row;

    let status = match status_db.as_str() {
        "archived" => crate::session::persistence::SessionStatus::Archived,
        _ => crate::session::persistence::SessionStatus::Active,
    };

    let transcript_path = match status {
        crate::session::persistence::SessionStatus::Active => data_dir
            .join("sessions")
            .join(format!("{session_id}.jsonl")),
        crate::session::persistence::SessionStatus::Archived => data_dir
            .join("archived_sessions")
            .join(format!("{session_id}.jsonl")),
    };

    let pending_messages = if transcript_path.exists() {
        read_transcript(&transcript_path)?
    } else {
        return Err(PersistenceError::NotFound(session_id.to_string()));
    };

    let last_message_id: Option<String> = None;
    let mut mode_state_val: crate::session::persistence::ReasoningModeState;
    let mode_val: String;
    if let Some(ref meta) = metadata {
        let v: serde_json::Value =
            serde_json::from_str(meta).map_err(PersistenceError::Serialization)?;
        mode_state_val = v
            .get("mode_state")
            .and_then(|x| serde_json::from_str(x.as_str().unwrap_or("{}")).ok())
            .unwrap_or_default();
        mode_val = v
            .get("mode")
            .and_then(|x| x.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "direct".to_string());
    } else {
        mode_state_val = crate::session::persistence::ReasoningModeState::default();
        mode_val = "direct".to_string();
    }

    let last_message_at = if last_msg_ts > 0 {
        Some(DateTime::from_timestamp(last_msg_ts, 0).unwrap_or_else(Utc::now))
    } else {
        None
    };

    Ok(Some(SessionCheckpoint {
        session_id: session_id.to_string(),
        last_message_id,
        mode_state: mode_state_val,
        pending_messages,
        mode: match mode_val.as_str() {
            "plan" => crate::session::persistence::ReasoningMode::Plan,
            "stream" => crate::session::persistence::ReasoningMode::Stream,
            "hidden" => crate::session::persistence::ReasoningMode::Hidden,
            _ => crate::session::persistence::ReasoningMode::Direct,
        },
        created_at: DateTime::from_timestamp(created_ts, 0).unwrap_or_else(Utc::now),
        updated_at: DateTime::from_timestamp(created_ts, 0).unwrap_or_else(Utc::now),
        ttl_seconds: 604800,
        status,
        last_message_at,
        message_count: msg_count as u64,
        channel: if channel.is_empty() {
            None
        } else {
            Some(channel)
        },
        chat_id: if chat_id.is_empty() {
            None
        } else {
            Some(chat_id)
        },
    }))
}

/// Read transcript entries from a .jsonl file.
fn read_transcript(
    path: &Path,
) -> Result<Vec<crate::session::persistence::PendingMessage>, PersistenceError> {
    #[derive(serde::Deserialize)]
    struct Entry {
        role: String,
        content: String,
        timestamp: DateTime<Utc>,
    }
    let content = std::fs::read_to_string(path).map_err(PersistenceError::Io)?;
    let mut messages = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: Entry = serde_json::from_str(line).map_err(PersistenceError::Serialization)?;
        messages.push(crate::session::persistence::PendingMessage {
            message_id: entry.role,
            content: entry.content,
            created_at: entry.timestamp,
            sent: true,
        });
    }
    Ok(messages)
}

/// Purge an archived checkpoint: delete its archived transcript and remove
/// the DB record.
pub fn do_purge(data_dir: &Path, session_id: &str) -> Result<(), PersistenceError> {
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    begin_immediate(&conn)?;

    let archived_path = data_dir
        .join("archived_sessions")
        .join(format!("{session_id}.jsonl"));
    delete_transcript(&archived_path);

    conn.execute("DELETE FROM sessions WHERE id = ?1", [session_id])
        .map_err(|e| {
            rollback(&conn);
            PersistenceError::Sqlite(e.to_string())
        })?;

    commit(&conn)
}

/// List all archived session IDs.
pub fn do_list_archived(data_dir: &Path) -> Result<Vec<String>, PersistenceError> {
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    let mut stmt = conn
        .prepare("SELECT id FROM sessions WHERE status = 'archived'")
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(ids)
}

/// List IDs of active sessions idle for at least `idle_minutes` milliseconds.
pub fn list_idle_sessions_inner(
    data_dir: &Path,
    idle_minutes: i64,
) -> Result<Vec<String>, PersistenceError> {
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let cutoff = Utc::now().timestamp_millis() - idle_minutes * 60 * 1000;
    let mut stmt = conn
        .prepare("SELECT id FROM sessions WHERE status = 'active' AND last_message_at < ?1")
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let ids: Vec<String> = stmt
        .query_map([cutoff], |row| row.get(0))
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// List IDs of archived sessions past their purge window.
pub fn list_expired_archived_sessions_inner(
    data_dir: &Path,
    purge_after_minutes: i64,
) -> Result<Vec<String>, PersistenceError> {
    if purge_after_minutes == 0 {
        return Ok(Vec::new());
    }
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let cutoff = Utc::now().timestamp_millis() - purge_after_minutes * 60 * 1000;
    let mut stmt = conn
        .prepare("SELECT id FROM sessions WHERE status = 'archived' AND archived_at < ?1")
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let ids: Vec<String> = stmt
        .query_map([cutoff], |row| row.get(0))
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// List IDs of active sessions for a specific agent/role idle for at least
/// `idle_minutes`.  `role` is a string such as "main_agent" or "sub_agent".
pub fn list_idle_sessions_for_agent_inner(
    data_dir: &Path,
    agent_id: &str,
    role: &str,
    idle_minutes: i64,
) -> Result<Vec<String>, PersistenceError> {
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let cutoff = Utc::now().timestamp_millis() - idle_minutes * 60 * 1000;
    let mut stmt = conn
        .prepare(
            "SELECT id FROM sessions \
             WHERE agent_id = ?1 AND role = ?2 AND status = 'active' \
             AND last_message_at < ?3",
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let ids: Vec<String> = stmt
        .query_map(params![agent_id, role, cutoff], |row| row.get(0))
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// List IDs of archived sessions for a specific agent/role past their purge
/// window.  `role` is a string such as "main_agent" or "sub_agent".
pub fn list_expired_archived_sessions_for_agent_inner(
    data_dir: &Path,
    agent_id: &str,
    role: &str,
    purge_after_minutes: i64,
) -> Result<Vec<String>, PersistenceError> {
    if purge_after_minutes == 0 {
        return Ok(Vec::new());
    }
    let db_path = data_dir.join("sessions.db");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let cutoff = Utc::now().timestamp_millis() - purge_after_minutes * 60 * 1000;
    let mut stmt = conn
        .prepare(
            "SELECT id FROM sessions \
             WHERE agent_id = ?1 AND role = ?2 AND status = 'archived' \
             AND archived_at < ?3",
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let ids: Vec<String> = stmt
        .query_map(params![agent_id, role, cutoff], |row| row.get(0))
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}
