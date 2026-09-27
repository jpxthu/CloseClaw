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

/// Archive a checkpoint with two-step crash-safe protocol.
///
/// Step A: set status to `migrating` (transcript still in `sessions/`)
/// Step B: move transcript file from `sessions/` to `archived_sessions/`
/// Step C: set status to `archived` with `archived_at` timestamp
///
/// Idempotent: returns no-op if status is already `archived`.
/// If status is `migrating` (crash between A and C), skips Step A
/// and continues from Step B.
pub fn do_archive(data_dir: &Path, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError> {
    let db_path = data_dir.join("sessions.sqlite");
    let conn = Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    // --- Step A: set status to `migrating` (or detect current state) ---
    begin_immediate(&conn)?;

    let current_status: Option<String> = conn
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            [&checkpoint.session_id],
            |row| row.get(0),
        )
        .ok();

    match current_status.as_deref() {
        Some("archived") => {
            // Already archived — idempotent no-op
            return commit(&conn).map(|_| ());
        }
        Some("migrating") => {
            // Crash between A and C — skip Step A, continue from Step B
            commit(&conn)?;
        }
        Some("active") => {
            // Normal path: set migrating
            conn.execute(
                "UPDATE sessions SET status = 'migrating' WHERE id = ?1",
                [&checkpoint.session_id],
            )
            .map_err(|e| {
                rollback(&conn);
                PersistenceError::Sqlite(e.to_string())
            })?;
            commit(&conn)?;
        }
        _ => {
            // Session doesn't exist — idempotent no-op
            return commit(&conn).map(|_| ());
        }
    }

    // --- Step B: move transcript file ---
    let src = data_dir
        .join("sessions")
        .join(format!("{}.jsonl", checkpoint.session_id));
    let dst = data_dir
        .join("archived_sessions")
        .join(format!("{}.jsonl", checkpoint.session_id));

    // File may already be at dst if crash occurred after move but before Step C
    if !dst.exists() {
        rename_transcript(&src, &dst)?;
    }

    // --- Step C: set status to `archived` ---
    begin_immediate(&conn)?;

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

    // Verify status is archived or migrating
    let status: String = match conn.query_row(
        "SELECT status FROM sessions WHERE id = ?1",
        [session_id],
        |row| row.get(0),
    ) {
        Ok(s) => s,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            rollback(&conn);
            return Err(PersistenceError::NotFound(session_id.to_string()));
        }
        Err(e) => return Err(PersistenceError::Sqlite(e.to_string())),
    };

    if status != "archived" && status != "migrating" {
        rollback(&conn);
        return Err(PersistenceError::NotFound(session_id.to_string()));
    }

    // Move transcript: from archived_sessions/ (if present) to sessions/
    // For migrating sessions, the file could be in either location.
    let archived_src = data_dir
        .join("archived_sessions")
        .join(format!("{session_id}.jsonl"));
    let active_dst = data_dir
        .join("sessions")
        .join(format!("{session_id}.jsonl"));

    if archived_src.exists() && !active_dst.exists() {
        rename_transcript(&archived_src, &active_dst)?;
    }

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

/// Raw column values of one `sessions` row, in SELECT column order.
///
/// Kept as a tuple so the row query and the destructure stay a pure
/// mechanical move of the original inline closure and pattern.
type SessionRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    Option<i64>,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// Stage ① of `load_checkpoint_inner`: run the row query and read all raw
/// columns. Returns `None` when the session row does not exist.
fn query_session_row(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<SessionRow>, PersistenceError> {
    let mut stmt = conn
        .prepare(
            "SELECT agent_id, role, channel, chat_id, status, title,
             last_message_at, created_at, archived_at, message_count, metadata, thread_id,
             sender_id, platform, peer_id, account_id, parent_session_id, depth,
             mined, dreaming_status, plan_state, mined_at, last_user_activity_at
             FROM sessions WHERE id = ?1",
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

    match stmt.query_row(params![session_id], |row| {
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
            row.get::<_, Option<i64>>(22)?,
        ))
    }) {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(PersistenceError::Sqlite(e.to_string())),
    }
}

/// Fields parsed from a metadata JSON blob.
///
/// `metadata` is `None` when the DB column was NULL (all defaults kept).
struct ParsedMetadata {
    mode_state: crate::persistence::ReasoningModeState,
    reasoning_mode_raw: String,
    user_appends: Vec<String>,
    session_mode: crate::persistence::SessionMode,
    outbound_pending: Vec<crate::persistence::PendingMessage>,
}

/// Stage ③ of `load_checkpoint_inner`: parse the snapshot metadata JSON
/// (session_mode / mode / user_appends / outbound_pending).
///
/// `None` metadata (NULL column) keeps all defaults.
fn parse_metadata(metadata: &Option<String>) -> Result<ParsedMetadata, PersistenceError> {
    #[allow(unused_mut)]
    let mut mode_state_val: crate::persistence::ReasoningModeState;
    let mode_val: String;
    let mut user_appends: Vec<String> = Vec::new();
    let mut outbound_pending: Vec<crate::persistence::PendingMessage> = Vec::new();
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
        user_appends = v
            .get("user_appends")
            .or_else(|| v.get("system_appends"))
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
    Ok(ParsedMetadata {
        mode_state: mode_state_val,
        reasoning_mode_raw: mode_val,
        user_appends,
        session_mode: session_mode_val,
        outbound_pending,
    })
}

/// State resolved from the raw `sessions` row (stage ② output of
/// `load_checkpoint_inner`): typed enums plus the transcript messages read
/// from disk.
struct ResolvedSessionState {
    depth: u32,
    mined: bool,
    dreaming_status: crate::persistence::DreamingStatus,
    status: crate::persistence::SessionStatus,
    transcript_messages_from_jsonl: Vec<crate::llm_session::SessionMessage>,
}

/// Stage ② of `load_checkpoint_inner`: resolve raw fields into types/enums
/// (depth / mined / dreaming_status / status) and read the transcript file
/// according to the resolved status.
fn resolve_session_state(
    data_dir: &Path,
    session_id: &str,
    status_db: &str,
    depth_str: Option<String>,
    mined_raw: &Option<String>,
    dreaming_status_raw: &Option<String>,
) -> Result<ResolvedSessionState, PersistenceError> {
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

    let status = match status_db {
        "archived" => crate::persistence::SessionStatus::Archived,
        "migrating" => crate::persistence::SessionStatus::Migrating,
        _ => crate::persistence::SessionStatus::Active,
    };

    let transcript_path = match status {
        crate::persistence::SessionStatus::Active => data_dir
            .join("sessions")
            .join(format!("{session_id}.jsonl")),
        crate::persistence::SessionStatus::Migrating => {
            // During migration, transcript could be in either location.
            // Prefer archived_sessions/ (file already moved) over sessions/
            // (file not yet moved).
            let archived = data_dir
                .join("archived_sessions")
                .join(format!("{session_id}.jsonl"));
            if archived.exists() {
                archived
            } else {
                data_dir
                    .join("sessions")
                    .join(format!("{session_id}.jsonl"))
            }
        }
        crate::persistence::SessionStatus::Archived => data_dir
            .join("archived_sessions")
            .join(format!("{session_id}.jsonl")),
    };

    let transcript_messages_from_jsonl = if transcript_path.exists() {
        read_transcript(&transcript_path)?
    } else {
        return Err(PersistenceError::NotFound(session_id.to_string()));
    };

    Ok(ResolvedSessionState {
        depth,
        mined,
        dreaming_status,
        status,
        transcript_messages_from_jsonl,
    })
}

/// Assemble the final `SessionCheckpoint` (stage ④ of
/// `load_checkpoint_inner`) from the resolved row state, the parsed snapshot
/// metadata and the remaining raw fields.
#[allow(clippy::too_many_arguments)]
fn build_checkpoint(
    session_id: &str,
    resolved: ResolvedSessionState,
    parsed_meta: ParsedMetadata,
    agent_id_str: String,
    role_str: String,
    channel: String,
    chat_id: String,
    thread_id: Option<String>,
    sender_id: Option<String>,
    platform_new: Option<String>,
    peer_id_new: Option<String>,
    account_id_new: Option<String>,
    parent_session_id: Option<String>,
    plan_state_raw: Option<String>,
    mined_at_raw: Option<i64>,
    last_user_activity_at_raw: Option<i64>,
    last_msg_ts: i64,
    created_ts: i64,
    msg_count: i64,
) -> SessionCheckpoint {
    let ResolvedSessionState {
        depth,
        mined,
        dreaming_status,
        status,
        transcript_messages_from_jsonl,
    } = resolved;
    let ParsedMetadata {
        mode_state: mode_state_val,
        reasoning_mode_raw: mode_val,
        user_appends,
        session_mode: session_mode_val,
        outbound_pending,
    } = parsed_meta;

    let last_message_at = if last_msg_ts > 0 {
        Some(DateTime::from_timestamp(last_msg_ts, 0).unwrap_or_else(Utc::now))
    } else {
        None
    };

    let last_message_id: Option<String> = None;
    let transcript_messages: Vec<crate::llm_session::SessionMessage> =
        transcript_messages_from_jsonl;
    SessionCheckpoint {
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
        user_appends,
        account_id: account_id_new,
        thread_id,
        reply_ref: None,
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
        last_user_activity_at: last_user_activity_at_raw
            .and_then(|ts| DateTime::from_timestamp(ts, 0)),
        progress_tool_calls: Vec::new(),
        approval_tool_calls: Vec::new(),
        plan_references: Vec::new(),
        session_mode: session_mode_val,
        pending_messages: transcript_messages,
        label: None,
        communication_config: None,
        snapshot_metas: Vec::new(),
        workflow_run: None,
        recovery_workflow_messages: Vec::new(),
        system_injection_appends: Vec::new(),
    }
}

/// Load a SessionCheckpoint from an open DB connection.
pub fn load_checkpoint_inner(
    conn: &Connection,
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<SessionCheckpoint>, PersistenceError> {
    // --- Stage ①: row query + raw column destructure ---
    let Some(row) = query_session_row(conn, session_id)? else {
        return Ok(None);
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
        last_user_activity_at_raw,
    ) = row;

    // --- Stage ②: raw fields → types/enums + transcript read ---
    let resolved = resolve_session_state(
        data_dir,
        session_id,
        &status_db,
        depth_str,
        &mined_raw,
        &dreaming_status_raw,
    )?;

    // --- Stage ③④: snapshot metadata JSON parsing + checkpoint assembly ---
    let parsed_meta = parse_metadata(&metadata)?;
    Ok(Some(build_checkpoint(
        session_id,
        resolved,
        parsed_meta,
        agent_id_str,
        role_str,
        channel,
        chat_id,
        thread_id,
        sender_id,
        platform_new,
        peer_id_new,
        account_id_new,
        parent_session_id,
        plan_state_raw,
        mined_at_raw,
        last_user_activity_at_raw,
        last_msg_ts,
        created_ts,
        msg_count,
    )))
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
             AND COALESCE(last_user_activity_at, last_message_at) < ?3",
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

/// Save a checkpoint to the database and write its transcript.
/// Extracted from SqliteStorage to keep sqlite.rs under 1000 lines.
pub fn save_checkpoint_inner(
    data_dir: &Path,
    checkpoint: &SessionCheckpoint,
) -> Result<(), PersistenceError> {
    let conn = Connection::open(data_dir.join("sessions.sqlite"))
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    let data = serialize_checkpoint_data(checkpoint)?;
    insert_session(&conn, checkpoint, &data)?;
    let transcript_path = data_dir
        .join("sessions")
        .join(format!("{}.jsonl", checkpoint.session_id));
    write_transcript(&transcript_path, checkpoint)?;
    Ok(())
}

/// Intermediate data produced by serializing checkpoint fields for SQL insertion.
struct CheckpointData<'a> {
    status: &'a str,
    role_str: &'a str,
    metadata_json: String,
    last_msg_ts: i64,
    dreaming_status_str: &'static str,
    mined_str: &'a str,
    mined_at_val: Option<i64>,
    plan_state_json: Option<String>,
    last_user_activity_ts: Option<i64>,
}

fn reasoning_mode_to_str(mode: crate::persistence::ReasoningMode) -> &'static str {
    match mode {
        crate::persistence::ReasoningMode::Direct => "direct",
        crate::persistence::ReasoningMode::Plan => "plan",
        crate::persistence::ReasoningMode::Stream => "stream",
        crate::persistence::ReasoningMode::Hidden => "hidden",
    }
}

/// Serialize checkpoint fields into strings and JSON values for SQL insertion.
fn serialize_checkpoint_data<'a>(
    checkpoint: &'a SessionCheckpoint,
) -> Result<CheckpointData<'a>, PersistenceError> {
    let status = match checkpoint.status {
        crate::persistence::SessionStatus::Active => "active",
        crate::persistence::SessionStatus::Migrating => "migrating",
        crate::persistence::SessionStatus::Archived => "archived",
    };
    let role_str = checkpoint
        .role
        .map(|r| match r {
            crate::persistence::AgentRole::MainAgent => "main_agent",
            crate::persistence::AgentRole::SubAgent => "sub_agent",
        })
        .unwrap_or("main_agent");
    let metadata_json = build_metadata_json(checkpoint)?;
    let last_msg_ts = checkpoint
        .last_message_at
        .map(|dt| dt.timestamp())
        .unwrap_or(0);
    let dreaming_status_str =
        crate::persistence::dreaming_status_to_db(&checkpoint.dreaming_status);
    let mined_str = if checkpoint.mined { "1" } else { "0" };
    let plan_state_json = checkpoint
        .plan_state
        .as_ref()
        .map(|ps| serde_json::to_string(ps).map_err(PersistenceError::Serialization))
        .transpose()?;
    let last_user_activity_ts = checkpoint.last_user_activity_at.map(|dt| dt.timestamp());
    Ok(CheckpointData {
        status,
        role_str,
        metadata_json,
        last_msg_ts,
        dreaming_status_str,
        mined_str,
        mined_at_val: checkpoint.mined_at,
        plan_state_json,
        last_user_activity_ts,
    })
}

/// Build the metadata JSON string from checkpoint fields.
fn build_metadata_json(checkpoint: &SessionCheckpoint) -> Result<String, PersistenceError> {
    let mode_state_json =
        serde_json::to_string(&checkpoint.mode_state).map_err(PersistenceError::Serialization)?;
    let pending_json = serde_json::to_string(&checkpoint.outbound_pending)
        .map_err(PersistenceError::Serialization)?;
    let user_appends_json =
        serde_json::to_string(&checkpoint.user_appends).map_err(PersistenceError::Serialization)?;
    serde_json::to_string(&serde_json::json!({
        "reasoning_mode": reasoning_mode_to_str(checkpoint.reasoning_mode),
        "mode_state": mode_state_json,
        "outbound_pending": pending_json,
        "user_appends": user_appends_json,
        "session_mode": checkpoint.session_mode.to_string(),
    }))
    .map_err(PersistenceError::Serialization)
}

/// Execute the INSERT OR REPLACE statement for a session checkpoint.
fn insert_session(
    conn: &Connection,
    checkpoint: &SessionCheckpoint,
    data: &CheckpointData,
) -> Result<(), PersistenceError> {
    conn.execute(
        "INSERT OR REPLACE INTO sessions
         (id, agent_id, role, channel, chat_id, status, title,
          last_message_at, created_at, archived_at, message_count, metadata, thread_id,
          sender_id, platform, peer_id, account_id, parent_session_id, depth,
          mined, dreaming_status, plan_state, mined_at, last_user_activity_at)
         VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7,
             ?8, ?9, ?10, ?11, ?12, ?13,
             ?14, ?15, ?16, ?17, ?18, ?19,
             ?20, ?21, ?22, ?23, ?24
         )",
        params![
            checkpoint.session_id,
            checkpoint.agent_id.as_deref().unwrap_or("unknown"),
            data.role_str,
            checkpoint.platform.as_deref().unwrap_or(""),
            checkpoint.peer_id.as_deref().unwrap_or(""),
            data.status,
            Option::<&str>::None,
            data.last_msg_ts,
            checkpoint.created_at.timestamp(),
            Option::<i64>::None,
            checkpoint.message_count as i64,
            data.metadata_json,
            checkpoint.thread_id.as_deref(),
            checkpoint.sender_id.as_deref(),
            checkpoint.platform.as_deref(),
            checkpoint.peer_id.as_deref(),
            checkpoint.account_id.as_deref(),
            checkpoint.parent_session_id.as_deref(),
            checkpoint.depth,
            data.mined_str,
            data.dreaming_status_str,
            data.plan_state_json.as_deref(),
            data.mined_at_val,
            data.last_user_activity_ts,
        ],
    )
    .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
    Ok(())
}
