//! SQLite archive operations helpers
//!
//! These helpers are used by SqliteStorage for archive/restore/purge/list
//! operations. Kept separate to keep sqlite.rs under 500 lines.

use crate::persistence::{PersistenceError, SessionCheckpoint};
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

/// Write transcript (pending_messages) to a .jsonl file.
pub fn write_transcript(
    path: &Path,
    checkpoint: &SessionCheckpoint,
) -> Result<(), PersistenceError> {
    let file = std::fs::File::create(path).map_err(PersistenceError::Io)?;
    let mut writer = std::io::BufWriter::new(file);
    for msg in &checkpoint.pending_messages {
        serde_json::to_writer(&mut writer, msg).map_err(PersistenceError::Serialization)?;
        use std::io::Write;
        writeln!(&mut writer).map_err(PersistenceError::Io)?;
    }
    Ok(())
}

/// Archive a checkpoint: move its active transcript to archived_sessions/
/// and mark the DB record as archived.
pub fn do_archive(data_dir: &Path, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError> {
    let db_path = data_dir.join("sessions.sqlite");
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
    let db_path = data_dir.join("sessions.sqlite");
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
             last_message_at, created_at, archived_at, message_count, metadata, thread_id,
             sender_id, platform, peer_id, account_id, parent_session_id, depth,
             mined, dreaming_status, plan_state, mined_at
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
            row.get::<_, Option<String>>(11)?,
            row.get::<_, Option<String>>(12)?,
            row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Option<String>>(15)?,
            row.get::<_, Option<String>>(16)?,
            row.get::<_, Option<String>>(17)?,
            row.get::<_, Option<String>>(18)?,
            row.get::<_, Option<String>>(19)?,
            row.get::<_, Option<String>>(20)?,
            row.get::<_, Option<i64>>(21)?,
        ))
    }) {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(PersistenceError::Sqlite(e.to_string())),
    };

    let (
        agent_id_str,
        role_str,
        channel,
        chat_id,
        status_db,
        _title,
        last_msg_ts,
        created_ts,
        _archived_ts,
        msg_count,
        metadata,
        thread_id,
        sender_id,
        platform_new,
        peer_id_new,
        account_id_new,
        parent_session_id,
        depth_str,
        mined_raw,
        dreaming_status_raw,
        plan_state_raw,
        mined_at_raw,
    ) = row;

    let depth: u32 = depth_str.and_then(|s| s.parse().ok()).unwrap_or(0);

    // mined: handle both INTEGER (0/1) and TEXT ("0"/"1") representations
    let mined: bool = mined_raw
        .as_deref()
        .map(|s| s != "0" && !s.is_empty() && s != "false")
        .unwrap_or(false);

    // dreaming_status: handle missing or empty string
    let dreaming_status = crate::persistence::dreaming_status_from_db(
        dreaming_status_raw.as_deref().unwrap_or("completed"),
    );

    let status = match status_db.as_str() {
        "archived" => crate::persistence::SessionStatus::Archived,
        _ => crate::persistence::SessionStatus::Active,
    };

    let transcript_path = match status {
        crate::persistence::SessionStatus::Active => data_dir
            .join("sessions")
            .join(format!("{session_id}.jsonl")),
        crate::persistence::SessionStatus::Archived => data_dir
            .join("archived_sessions")
            .join(format!("{session_id}.jsonl")),
    };

    let transcript_messages_from_jsonl = if transcript_path.exists() {
        read_transcript(&transcript_path)?
    } else {
        return Err(PersistenceError::NotFound(session_id.to_string()));
    };

    let last_message_id: Option<String> = None;
    #[allow(unused_mut)]
    let mut mode_state_val: crate::persistence::ReasoningModeState;
    let mode_val: String;
    let mut system_appends: Vec<String> = Vec::new();
    let mut outbound_pending: Vec<crate::persistence::PendingMessage> = Vec::new();
    let transcript_messages: Vec<crate::llm_session::SessionMessage> =
        transcript_messages_from_jsonl;
    let mut session_mode_val: crate::persistence::SessionMode =
        crate::persistence::SessionMode::default();
    if let Some(ref meta) = metadata {
        let v: serde_json::Value =
            serde_json::from_str(meta).map_err(PersistenceError::Serialization)?;
        mode_state_val = v
            .get("mode_state")
            .and_then(|x| serde_json::from_str(x.as_str().unwrap_or("{}")).ok())
            .unwrap_or_default();
        mode_val = v
            .get("reasoning_mode")
            .or_else(|| v.get("mode"))
            .and_then(|x| x.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "direct".to_string());
        system_appends = v
            .get("system_appends")
            .and_then(|x| serde_json::from_str(x.as_str().unwrap_or("[]")).ok())
            .unwrap_or_default();
        if let Some(mode_str) = v.get("session_mode").and_then(|x| x.as_str()) {
            session_mode_val =
                crate::persistence::SessionMode::from_str_opt(mode_str).unwrap_or_default();
        }
        outbound_pending = v
            .get("outbound_pending")
            .and_then(|x| serde_json::from_str(x.as_str().unwrap_or("[]")).ok())
            .unwrap_or_default();
    } else {
        mode_state_val = crate::persistence::ReasoningModeState::default();
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
        outbound_pending,
        reasoning_mode: match mode_val.as_str() {
            "plan" => crate::persistence::ReasoningMode::Plan,
            "stream" => crate::persistence::ReasoningMode::Stream,
            "hidden" => crate::persistence::ReasoningMode::Hidden,
            _ => crate::persistence::ReasoningMode::Direct,
        },
        created_at: DateTime::from_timestamp(created_ts, 0).unwrap_or_else(Utc::now),
        updated_at: DateTime::from_timestamp(created_ts, 0).unwrap_or_else(Utc::now),
        ttl_seconds: 604800,
        status,
        last_message_at,
        message_count: msg_count as u64,
        platform: {
            let has_new = platform_new.as_deref().is_some_and(|s| !s.is_empty());
            if has_new {
                platform_new
            } else if channel.is_empty() {
                None
            } else {
                Some(channel)
            }
        },
        peer_id: {
            let has_new = peer_id_new.as_deref().is_some_and(|s| !s.is_empty());
            if has_new {
                peer_id_new
            } else if chat_id.is_empty() {
                None
            } else {
                Some(chat_id)
            }
        },
        agent_id: if agent_id_str.is_empty() {
            None
        } else {
            Some(agent_id_str)
        },
        role: match role_str.as_str() {
            "main_agent" => Some(crate::persistence::AgentRole::MainAgent),
            "sub_agent" => Some(crate::persistence::AgentRole::SubAgent),
            _ => None,
        },
        reasoning_level: crate::persistence::ReasoningLevel::default(),
        system_appends,
        account_id: account_id_new,
        thread_id,
        sender_id,
        parent_session_id,
        depth,
        mined,
        mined_at: mined_at_raw,
        dreaming_status,
        effective_max_spawn_depth: None,
        pending_operations: Vec::new(),
        recovery_notification: None,
        pending_tool_failures: Vec::new(),
        verbosity_level: closeclaw_common::VerbosityLevel::default(),
        plan_state: plan_state_raw.and_then(|s| serde_json::from_str(&s).ok()),
        progress_tool_calls: Vec::new(),
        approval_tool_calls: Vec::new(),
        plan_references: Vec::new(),
        session_mode: session_mode_val,
        pending_messages: transcript_messages,
        label: None,
        communication_config: None,
        spawn_mode: None,
        snapshot_metas: Vec::new(),
    }))
}

/// Read transcript (pending_messages) from a .jsonl file.
fn read_transcript(
    path: &Path,
) -> Result<Vec<crate::llm_session::SessionMessage>, PersistenceError> {
    let content = std::fs::read_to_string(path).map_err(PersistenceError::Io)?;
    let mut messages = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let msg: crate::llm_session::SessionMessage =
            serde_json::from_str(line).map_err(PersistenceError::Serialization)?;
        messages.push(msg);
    }
    Ok(messages)
}

/// Purge an archived checkpoint: delete its archived transcript and remove
/// the DB record.
pub fn do_purge(data_dir: &Path, session_id: &str) -> Result<(), PersistenceError> {
    let db_path = data_dir.join("sessions.sqlite");
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
    let db_path = data_dir.join("sessions.sqlite");
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
    let db_path = data_dir.join("sessions.sqlite");
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
    let db_path = data_dir.join("sessions.sqlite");
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
    let db_path = data_dir.join("sessions.sqlite");
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
    let db_path = data_dir.join("sessions.sqlite");
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
