//! SQLite storage backend for session persistence
//!
//! This backend stores session checkpoints in a local SQLite database,
//! suitable for single-node deployments without Redis.

mod archive_support;

#[cfg(test)]
mod bug904_tests;
#[cfg(test)]
mod tests;

use crate::session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio::task::spawn_blocking;

/// SQLite storage backend
#[derive(Debug)]
pub struct SqliteStorage {
    data_dir: PathBuf,
}
impl SqliteStorage {
    /// Create a new SqliteStorage instance
    ///
    /// Creates the data directory structure and initializes the SQLite database.
    ///
    /// # Errors
    /// Returns `PersistenceError::Sqlite` if directory creation or DB init fails.
    pub fn new(data_dir: &Path) -> Result<Self, PersistenceError> {
        let data_dir = data_dir.to_path_buf();

        // Create directory structure: sessions/ and archived_sessions/
        let sessions_dir = data_dir.join("sessions");
        let archived_dir = data_dir.join("archived_sessions");
        std::fs::create_dir_all(&sessions_dir)
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        std::fs::create_dir_all(&archived_dir)
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

        // Migrate legacy `sessions.db` to `sessions.sqlite` (per design doc)
        let legacy_db_path = data_dir.join("sessions.db");
        let db_path = data_dir.join("sessions.sqlite");
        if legacy_db_path.exists() {
            std::fs::rename(&legacy_db_path, &db_path)
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        }

        // Open or create SQLite database
        let conn =
            Connection::open(&db_path).map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

        // Initialize schema
        Self::init_schema(&conn)?;

        Ok(Self { data_dir })
    }
    /// Initialize the database schema
    fn init_schema(conn: &Connection) -> Result<(), PersistenceError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
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
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_status
                ON sessions(status);

            CREATE INDEX IF NOT EXISTS idx_sessions_last_message_at
                ON sessions(last_message_at);

            CREATE INDEX IF NOT EXISTS idx_sessions_agent_id
                ON sessions(agent_id);

            CREATE INDEX IF NOT EXISTS idx_sessions_archived_at
                ON sessions(archived_at)
                WHERE archived_at IS NOT NULL;

            CREATE INDEX IF NOT EXISTS idx_sessions_agent_role
                ON sessions(agent_id, role);
            "#,
        )
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

        // Idempotent migration: only add thread_id column if it doesn't already exist.
        let column_exists: bool = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(sessions)")
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            let has_column = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
                .filter_map(|r| r.ok())
                .any(|name| name == "thread_id");
            has_column
        };
        if !column_exists {
            conn.execute("ALTER TABLE sessions ADD COLUMN thread_id TEXT", [])
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
        }

        Ok(())
    }
    /// Returns the data directory path
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// List IDs of active sessions idle for at least `idle_minutes`.
    pub async fn list_idle_sessions(
        &self,
        idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        spawn_blocking(move || archive_support::list_idle_sessions_inner(&data_dir, idle_minutes))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List IDs of archived sessions past their purge window.
    pub async fn list_expired_archived_sessions(
        &self,
        purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        spawn_blocking(move || {
            archive_support::list_expired_archived_sessions_inner(&data_dir, purge_after_minutes)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List IDs of active sessions for a specific agent/role idle for at least
    /// `idle_minutes`.  `role` is a string such as "main_agent" or "sub_agent".
    pub async fn list_idle_sessions_for_agent(
        &self,
        agent_id: &str,
        role: &str,
        idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let agent_id = agent_id.to_string();
        let role = role.to_string();
        spawn_blocking(move || {
            archive_support::list_idle_sessions_for_agent_inner(
                &data_dir,
                &agent_id,
                &role,
                idle_minutes,
            )
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List IDs of archived sessions for a specific agent/role past their purge
    /// window.  `role` is a string such as "main_agent" or "sub_agent".
    pub async fn list_expired_archived_sessions_for_agent(
        &self,
        agent_id: &str,
        role: &str,
        purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let agent_id = agent_id.to_string();
        let role = role.to_string();
        spawn_blocking(move || {
            archive_support::list_expired_archived_sessions_for_agent_inner(
                &data_dir,
                &agent_id,
                &role,
                purge_after_minutes,
            )
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }
}

impl std::fmt::Display for SqliteStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SqliteStorage({})", self.data_dir.display())
    }
}

impl Clone for SqliteStorage {
    fn clone(&self) -> Self {
        Self {
            data_dir: self.data_dir.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers for row <-> SessionCheckpoint conversion
// ---------------------------------------------------------------------------

/// Convert SessionStatus to/from database string representation
fn status_to_db(s: &crate::session::persistence::SessionStatus) -> &'static str {
    match s {
        crate::session::persistence::SessionStatus::Active => "active",
        crate::session::persistence::SessionStatus::Archived => "archived",
    }
}

/// Convert ReasonMode to/from database string representation
fn mode_to_db(m: &crate::session::persistence::ReasoningMode) -> &'static str {
    match m {
        crate::session::persistence::ReasoningMode::Direct => "direct",
        crate::session::persistence::ReasoningMode::Plan => "plan",
        crate::session::persistence::ReasoningMode::Stream => "stream",
        crate::session::persistence::ReasoningMode::Hidden => "hidden",
    }
}

// PersistenceService implementation — Part 1: core read/write methods
// ---------------------------------------------------------------------------

impl SqliteStorage {
    /// Load a SessionCheckpoint from an open DB connection.
    /// Used by `load_checkpoint`.
    fn load_checkpoint_inner(
        conn: &Connection,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        archive_support::load_checkpoint_inner(conn, data_dir, session_id)
    }
}

#[async_trait]
impl PersistenceService for SqliteStorage {
    /// Save a session checkpoint to the database and write its transcript
    /// to `sessions/<id>.jsonl`.
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let checkpoint = checkpoint.clone();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let status = status_to_db(&checkpoint.status);
            let _mode = mode_to_db(&checkpoint.mode);
            let mode_state_json = serde_json::to_string(&checkpoint.mode_state)
                .map_err(PersistenceError::Serialization)?;
            let pending_json = serde_json::to_string(&checkpoint.pending_messages)
                .map_err(PersistenceError::Serialization)?;
            let system_appends_json = serde_json::to_string(&checkpoint.system_appends)
                .map_err(PersistenceError::Serialization)?;
            let metadata_json = json!({
                "mode": mode_to_db(&checkpoint.mode),
                "mode_state": mode_state_json,
                "pending_messages": pending_json,
                "system_appends": system_appends_json,
            })
            .to_string();

            let last_msg_ts = checkpoint
                .last_message_at
                .map(|dt| dt.timestamp())
                .unwrap_or(0);

            conn.execute(
                "INSERT OR REPLACE INTO sessions
                 (id, agent_id, role, channel, chat_id, status, title,
                  last_message_at, created_at, archived_at, message_count, metadata, thread_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    checkpoint.session_id,
                    checkpoint.agent_id.as_deref().unwrap_or("unknown"),
                    checkpoint
                        .role
                        .map(|r| match r {
                            crate::session::persistence::AgentRole::MainAgent => "main_agent",
                            crate::session::persistence::AgentRole::SubAgent => "sub_agent",
                        })
                        .unwrap_or("main_agent"),
                    checkpoint.channel.as_deref().unwrap_or(""),
                    checkpoint.chat_id.as_deref().unwrap_or(""),
                    status,
                    Option::<&str>::None, // title
                    last_msg_ts,
                    checkpoint.created_at.timestamp(),
                    Option::<i64>::None, // archived_at
                    checkpoint.message_count as i64,
                    metadata_json,
                    checkpoint.thread_id.as_deref(),
                ],
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            // Write transcript to sessions/<id>.jsonl
            let transcript_path = data_dir
                .join("sessions")
                .join(format!("{}.jsonl", checkpoint.session_id));
            archive_support::write_transcript(&transcript_path, &checkpoint)?;

            Ok(())
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Load a session checkpoint from the database and reconstruct
    /// pending_messages from the transcript file.
    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;
            Self::load_checkpoint_inner(&conn, &data_dir, &session_id)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Delete a session checkpoint from the database and remove its
    /// transcript file (active or archived).
    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            conn.execute(
                "DELETE FROM sessions WHERE id = ?1",
                rusqlite::params![session_id],
            )
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            // Remove transcript from sessions/ then archived_sessions/
            let active_path = data_dir
                .join("sessions")
                .join(format!("{session_id}.jsonl"));
            archive_support::delete_transcript(&active_path);

            let archived_path = data_dir
                .join("archived_sessions")
                .join(format!("{session_id}.jsonl"));
            archive_support::delete_transcript(&archived_path);

            Ok(())
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List all active session IDs.
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();

        spawn_blocking(move || {
            let conn = Connection::open(data_dir.join("sessions.sqlite"))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let mut stmt = conn
                .prepare("SELECT id FROM sessions WHERE status = 'active'")
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?;

            let ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();

            Ok(ids)
        })
        .await
        .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Archive a session: move its transcript to archived_sessions/ and mark
    /// the DB record as archived. Idempotent if the session is not active.
    async fn archive_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let checkpoint = checkpoint.clone();

        spawn_blocking(move || archive_support::do_archive(&data_dir, &checkpoint))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Restore an archived session: move transcript back to sessions/ and mark
    /// the DB record as active.
    async fn restore_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || archive_support::do_restore(&data_dir, &session_id))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Permanently delete an archived checkpoint and its transcript.
    async fn purge_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let data_dir = self.data_dir.clone();
        let session_id = session_id.to_string();

        spawn_blocking(move || archive_support::do_purge(&data_dir, &session_id))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// List all archived session IDs.
    async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let data_dir = self.data_dir.clone();

        spawn_blocking(move || archive_support::do_list_archived(&data_dir))
            .await
            .map_err(|e| PersistenceError::Sqlite(e.to_string()))?
    }

    /// Invalidate a session (no-op for SQLite backend).
    async fn invalidate_session(&self, _session_id: &str) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        agent_id: &str,
        role: crate::session::persistence::AgentRole,
        idle_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let role_str = match role {
            crate::session::persistence::AgentRole::MainAgent => "main_agent",
            crate::session::persistence::AgentRole::SubAgent => "sub_agent",
        };
        self.list_idle_sessions_for_agent(agent_id, role_str, idle_minutes)
            .await
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        agent_id: &str,
        role: crate::session::persistence::AgentRole,
        purge_after_minutes: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let role_str = match role {
            crate::session::persistence::AgentRole::MainAgent => "main_agent",
            crate::session::persistence::AgentRole::SubAgent => "sub_agent",
        };
        self.list_expired_archived_sessions_for_agent(agent_id, role_str, purge_after_minutes)
            .await
    }
}
